//! Finds font files on this machine for the faces a design uses. Font files are untrusted: only
//! the sfnt `name` table of .ttf/.otf/.ttc is read, within fixed budgets, and WOFF/WOFF2 are
//! matched by file name alone. Nothing is copied, downloaded or loaded.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Directory entries visited, font files opened and bytes read, per lookup.
const MAX_ENTRIES: usize = 100_000;
const MAX_FONT_FILES: usize = 20_000;
const BYTE_BUDGET: u64 = 64 << 20;
const MAX_DEPTH: usize = 8;
/// Per font: table directory entries, `name` table bytes, records and kept string bytes.
const MAX_TABLES: usize = 256;
const MAX_NAME_TABLE: usize = 1 << 20;
const MAX_RECORDS: usize = 4096;
const MAX_STRING: usize = 256;
/// Fonts per collection and matches kept per face.
const MAX_COLLECTION: usize = 64;
const MAX_HITS: usize = 3;

/// A face the design uses.
pub struct Want<'a> {
    pub family: &'a str,
    pub style: &'a str,
    pub post_script_name: &'a str,
}

pub struct Hit {
    pub path: PathBuf,
    /// `postScriptName`, `family+style` (the font's own name table) or `filename` (WOFF/WOFF2).
    pub by: &'static str,
    pub project: bool,
}

#[derive(Default)]
pub struct Summary {
    pub searched: Vec<PathBuf>,
    pub files_read: usize,
    /// A budget stopped the scan; faces without a hit may still exist.
    pub truncated: bool,
}

/// Name records of one font: PostScript names, and (family, subfamily) pairs from name ids 1/2
/// and 16/17.
#[derive(Default)]
struct Names {
    ps: Vec<String>,
    pairs: [(Vec<String>, Vec<String>); 2],
}

fn be(a: &[u8], at: usize, n: usize) -> Option<u64> {
    let b = a.get(at..at.checked_add(n)?)?;
    Some(b.iter().fold(0, |x, &y| x << 8 | u64::from(y)))
}

/// `n` bytes at `off`, only when inside the file and the byte budget (checked before reading).
fn at(f: &mut File, len: u64, off: u64, n: usize, budget: &mut u64) -> Option<Vec<u8>> {
    if off.checked_add(n as u64)? > len || n as u64 > *budget {
        return None;
    }
    *budget -= n as u64;
    let mut buf = vec![0; n];
    f.seek(SeekFrom::Start(off)).ok()?;
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn string(platform: u64, raw: &[u8]) -> Option<String> {
    match platform {
        0 | 3 => char::decode_utf16(
            raw.chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]])),
        )
        .collect::<Result<String, _>>()
        .ok(),
        1 => raw
            .is_ascii()
            .then(|| String::from_utf8_lossy(raw).into_owned()),
        _ => None,
    }
}

/// The names of the sfnt starting at `base`; None when it is not a valid sfnt or over budget.
fn sfnt_names(f: &mut File, len: u64, base: u64, budget: &mut u64) -> Option<Names> {
    let head = at(f, len, base, 12, budget)?;
    if !matches!(be(&head, 0, 4)?, 0x0001_0000 | 0x4F54_544F | 0x7472_7565) {
        return None;
    }
    let tables = be(&head, 4, 2)? as usize;
    if tables == 0 || tables > MAX_TABLES {
        return None;
    }
    let dir = at(f, len, base + 12, tables * 16, budget)?;
    let (off, size) = (0..tables)
        .map(|i| &dir[i * 16..])
        .find(|e| e.starts_with(b"name"))
        .and_then(|e| Some((be(e, 8, 4)?, be(e, 12, 4)? as usize)))?;
    if !(6..=MAX_NAME_TABLE).contains(&size) {
        return None;
    }
    let table = at(f, len, off, size, budget)?;
    let (count, strings) = (be(&table, 2, 2)? as usize, be(&table, 4, 2)? as usize);
    if count > MAX_RECORDS || 6 + count * 12 > size {
        return None;
    }
    let mut names = Names::default();
    for r in 0..count {
        let at = 6 + r * 12;
        let (platform, id) = (be(&table, at, 2)?, be(&table, at + 6, 2)?);
        let (n, o) = (
            be(&table, at + 8, 2)? as usize,
            strings + be(&table, at + 10, 2)? as usize,
        );
        let slot = match id {
            6 => &mut names.ps,
            1 | 16 => &mut names.pairs[usize::from(id == 16)].0,
            2 | 17 => &mut names.pairs[usize::from(id == 17)].1,
            _ => continue,
        };
        // A record pointing outside the table is dropped, not trusted.
        if n > MAX_STRING || slot.len() >= 4 {
            continue;
        }
        if let Some(s) = table.get(o..o + n).and_then(|raw| string(platform, raw))
            && !slot.contains(&s)
        {
            slot.push(s);
        }
    }
    Some(names)
}

/// Every font in a .ttf/.otf/.ttc file.
fn file_names(path: &Path, budget: &mut u64) -> Vec<Names> {
    let Ok(mut f) = File::open(path) else {
        return vec![];
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return vec![];
    };
    let Some(head) = at(&mut f, len, 0, 12, budget) else {
        return vec![];
    };
    if &head[..4] != b"ttcf" {
        return sfnt_names(&mut f, len, 0, budget).into_iter().collect();
    }
    let n = be(&head, 8, 4).unwrap_or(0) as usize;
    if n > MAX_COLLECTION {
        return vec![];
    }
    let Some(offsets) = at(&mut f, len, 12, n * 4, budget) else {
        return vec![];
    };
    (0..n)
        .filter_map(|i| sfnt_names(&mut f, len, be(&offsets, i * 4, 4)?, budget))
        .collect()
}

fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The wanted faces by normalized name, so each file costs a few lookups however many faces the
/// design uses.
#[derive(Default)]
struct Index {
    ps: HashMap<String, Vec<usize>>,
    family_style: HashMap<(String, String), Vec<usize>>,
    stem: HashMap<String, Vec<usize>>,
}

impl Index {
    fn new(wants: &[Want]) -> Self {
        let mut index = Self::default();
        let key = |s: &str| s.to_lowercase();
        for (i, w) in wants.iter().enumerate() {
            if !w.post_script_name.is_empty() {
                index.ps.entry(key(w.post_script_name)).or_default().push(i);
            }
            if !w.family.is_empty() && !w.style.is_empty() {
                let k = (key(w.family), key(w.style));
                index.family_style.entry(k).or_default().push(i);
            }
            for name in [
                w.post_script_name.to_owned(),
                format!("{}{}", w.family, w.style),
            ] {
                let name = squash(&name);
                if !name.is_empty() {
                    index.stem.entry(name).or_default().push(i);
                }
            }
        }
        index
    }

    /// Wanted faces named by a font's own name table, with the first way each matched.
    fn by_names(&self, fonts: &[Names], found: &mut BTreeMap<usize, &'static str>) {
        for names in fonts {
            for p in &names.ps {
                for &i in self.ps.get(&p.to_lowercase()).into_iter().flatten() {
                    found.entry(i).or_insert("postScriptName");
                }
            }
            for (family, sub) in &names.pairs {
                for (f, s) in family.iter().flat_map(|f| sub.iter().map(move |s| (f, s))) {
                    let k = (f.to_lowercase(), s.to_lowercase());
                    for &i in self.family_style.get(&k).into_iter().flatten() {
                        found.entry(i).or_insert("family+style");
                    }
                }
            }
        }
    }
}

/// Per `wants` entry, up to `MAX_HITS` files from `roots` (project roots marked `true`, searched
/// first by the caller's order).
pub fn lookup(roots: &[(PathBuf, bool)], wants: &[Want]) -> (Vec<Vec<Hit>>, Summary) {
    let mut hits: Vec<Vec<Hit>> = wants.iter().map(|_| vec![]).collect();
    let index = Index::new(wants);
    let mut summary = Summary::default();
    let (mut entries, mut budget) = (0, BYTE_BUDGET);
    'roots: for (root, project) in roots {
        if !root.is_dir() {
            continue;
        }
        summary.searched.push(root.clone());
        // A project folder is the caller's read boundary: links in it must stay inside.
        let real_root = match (*project, std::fs::canonicalize(root)) {
            (true, Ok(real)) => Some(real),
            (true, Err(_)) => continue,
            _ => None,
        };
        let mut stack = vec![(root.clone(), 0)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            // Collect no more than the entry budget has left (+1 to notice the excess).
            let mut children: Vec<_> = read.flatten().take(MAX_ENTRIES - entries + 1).collect();
            children.sort_by_key(|c| c.file_name());
            for child in children {
                entries += 1;
                if entries > MAX_ENTRIES || summary.files_read >= MAX_FONT_FILES {
                    summary.truncated = true;
                    break 'roots;
                }
                let path = child.path();
                // File types do not follow links: a linked directory is never entered.
                let Ok(kind) = child.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    if depth < MAX_DEPTH {
                        stack.push((path, depth + 1));
                    } else {
                        summary.truncated = true;
                    }
                    continue;
                }
                if !path.is_file() {
                    continue;
                }
                if let Some(real_root) = &real_root
                    && kind.is_symlink()
                    && !std::fs::canonicalize(&path).is_ok_and(|real| real.starts_with(real_root))
                {
                    continue;
                }
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase);
                let web = match ext.as_deref() {
                    Some("ttf" | "otf" | "ttc" | "otc") => false,
                    Some("woff" | "woff2") => true,
                    _ => continue,
                };
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let fonts = if web {
                    vec![]
                } else {
                    summary.files_read += 1;
                    file_names(&path, &mut budget)
                };
                let mut found = BTreeMap::new();
                if web {
                    for &i in index.stem.get(&squash(stem)).into_iter().flatten() {
                        found.insert(i, "filename");
                    }
                } else {
                    index.by_names(&fonts, &mut found);
                }
                for (i, by) in found {
                    if hits[i].len() < MAX_HITS {
                        hits[i].push(Hit {
                            path: path.clone(),
                            by,
                            project: *project,
                        });
                    }
                }
                // Too little left for another name table: faces not found may still have a file.
                if budget < MAX_NAME_TABLE as u64 {
                    summary.truncated = true;
                    break 'roots;
                }
            }
        }
    }
    (hits, summary)
}

/// The user and system font folders of this platform.
pub fn system_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut dirs = vec![];
    #[cfg(target_os = "macos")]
    {
        dirs.extend(env("HOME").map(|h| h.join("Library/Fonts")));
        dirs.extend(
            ["/Library/Fonts", "/System/Library/Fonts"].map(PathBuf::from), // incl. Supplemental
        );
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        dirs.extend(env("HOME").map(|h| h.join(".local/share/fonts")));
        dirs.extend(env("HOME").map(|h| h.join(".fonts")));
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
    }
    #[cfg(windows)]
    {
        dirs.extend(env("WINDIR").map(|w| w.join("Fonts")));
        dirs.extend(env("LOCALAPPDATA").map(|l| l.join("Microsoft/Windows/Fonts")));
    }
    let _ = env;
    dirs
}

/// A CSS `@font-face` for the first project hit, `src` relative to the project folder `root`.
/// None for a collection, which a browser cannot load as one face.
/// `font-weight` is left out when the design recorded none, rather than claiming 400.
pub fn font_face(w: &Want, weight: Option<u64>, hit: &Hit, root: &Path) -> Option<String> {
    let format = match hit
        .path
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "woff2" => "woff2",
        "woff" => "woff",
        "ttf" => "truetype",
        "otf" => "opentype",
        _ => return None,
    };
    let rel: Vec<_> = hit
        .path
        .strip_prefix(root)
        .ok()?
        .components()
        .map(|c| {
            c.as_os_str()
                .to_string_lossy()
                .bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                        (b as char).to_string()
                    }
                    _ => format!("%{b:02X}"),
                })
                .collect::<String>()
        })
        .collect();
    let lower = w.style.to_ascii_lowercase();
    let italic = ["italic", "oblique"].iter().any(|k| lower.contains(k));
    // A CSS string: escape what ends it, and a newline as the escape `\A `.
    let family = w
        .family
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\A ")
        .replace('\r', "");
    Some(format!(
        "@font-face {{\n  font-family: \"{family}\";\n{}  font-style: {};\n  src: url(\"{}\") format(\"{format}\");\n  font-display: swap;\n}}",
        weight.map_or(String::new(), |w| format!("  font-weight: {w};\n")),
        if italic { "italic" } else { "normal" },
        rel.join("/"),
    ))
}

#[cfg(test)]
pub(crate) mod tests;
