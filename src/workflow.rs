//! MCP access to the same workflow sources shipped to Pi and skill users.

const SKILL: &str = include_str!("../skills/figlens/SKILL.md");
pub const TOPICS: &[&str] = &[
    "build",
    "audit",
    "images",
    "plan",
    "planning-details",
    "yolo",
];

pub const INSTRUCTIONS: &str = "Read local Figma .fig snapshots with figlens. Resolve the requested screen from names or visible text using find/tree, or search for copy inside instance overrides, properties and swaps; ask about recognizable choices only when materially ambiguous, and resolve node IDs yourself. Use inspect for compact layout/text, nodes for full styles/masks/component data, variables for tokens, export for reference pixels or assets, and verify to check a bundle against its manifest, list its font faces and read its handoff preflight findings. For implementation, call workflow(topic=build); for comparing existing UI with a design or correcting layout differences, call workflow(topic=audit); for implementation planning or careful reproduction, also load plan; for website image preparation, load images. Load planning-details only for page/multi-page scope or unresolved dependencies. Load yolo only when the user explicitly requests autonomous implementation and temporary-plan cleanup. A plan-only request ends with a plan, and so does a request to plan the whole scope before implementing: detail every slice, then wait for approval; an authorized build continues through implementation, visual checks and corrections using the project's existing architecture. Simple questions and exports use the design tools directly. Ordinary design questions search the file directly and create nothing. Only when the user asks to create or enable an index for a design, index prepare its file with the scope to search (figlens picks the storage; no setup step), poll jobs status until succeeded, reporting queued, running or failed jobs as such, then search with workspaceId and reuse that workspace afterwards. Run index reset or replaceUnavailable only on the user's explicit request. Workflow reads return guidance, not executed work; host tools and permissions govern project edits, browser checks and cleanup. Check isError and content on failed calls. Native Dev Mode annotations and font files are unavailable; report material gaps rather than guessing.";

const MCP_PREAMBLE: &str = "Use this guidance in the current agent context and within the user's authorized scope. It does not execute work or grant permissions. Use the configured figlens MCP tools with absolute file/output paths; CLI examples describe equivalent tool arguments. Project inspection/edits, directory creation, bundle extraction and browser checks require the host's own tools. Retrieve another topic only for missing guidance needed now: images covers image delivery; build covers design reading and project reuse. Links beginning workflow: name another topic of this tool, with an optional section anchor; no local skill installation is needed.\n\n";
const CLI_PREAMBLE: &str = "Use this guidance in the current agent context and within the user's authorized scope. It does not execute work or grant permissions. Run the figlens commands shown; MCP argument names map to the same CLI arguments. Project inspection/edits, directory creation, bundle extraction and browser checks require the host's own tools. Retrieve another topic only for missing guidance needed now: images covers image delivery; build covers design reading and project reuse. A link beginning workflow: names another topic: run `figlens workflow <topic>`; no local skill installation is needed.\n\n";

pub fn read(topic: &str) -> Result<String, String> {
    compose(topic, MCP_PREAMBLE)
}

/// The same guidance for an agent running the CLI without MCP or an installed skill.
pub fn read_cli(topic: &str) -> Result<String, String> {
    compose(topic, CLI_PREAMBLE)
}

fn compose(topic: &str, preamble: &str) -> Result<String, String> {
    let headings: &[&str] = match topic {
        "build" => &[
            "Where outputs go",
            "Building a frame",
            "Verification evidence",
            "Font and asset preflight",
            "Link back to Figma",
            "Reading the output",
            "Known gaps",
        ],
        "audit" => &["Where outputs go", "Verification evidence", "Known gaps"],
        "images" => &["Where outputs go", "Images and slices", "Known gaps"],
        "plan" | "planning-details" | "yolo" => &[],
        _ => {
            return Err(format!(
                "unknown workflow topic {topic:?}; choose {}",
                TOPICS.join(", ")
            ));
        }
    };
    let mut text = String::from(preamble);
    match topic {
        "audit" => text.push_str(include_str!("../skills/figlens/references/audit.md")),
        "plan" => text.push_str(include_str!("../skills/figlens/references/planning.md")),
        "planning-details" => text.push_str(include_str!(
            "../skills/figlens/references/planning-details.md"
        )),
        "yolo" => text.push_str(include_str!("../skills/figlens/references/yolo.md")),
        _ => {}
    }
    let skill = SKILL.replace("\r\n", "\n");
    for heading in headings {
        let marker = format!("## {heading}\n");
        let start = skill
            .find(&marker)
            .ok_or_else(|| format!("embedded workflow section missing: {heading}"))?;
        let section = &skill[start..];
        text.push_str(
            section
                .split_once("\n## ")
                .map_or(section, |(first, _)| first),
        );
        text.push_str("\n\n");
    }
    for (file, topic) in [
        ("../SKILL.md", "build"),
        ("audit.md", "audit"),
        ("planning.md", "plan"),
        ("planning-details.md", "planning-details"),
        ("yolo.md", "yolo"),
    ] {
        text = text.replace(
            &format!("(references/{file}"),
            &format!("(workflow:{topic}"),
        );
        text = text.replace(&format!("({file}"), &format!("(workflow:{topic}"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_and_build_share_evidence_rules_across_cli_and_mcp() {
        let marker = "## Verification evidence\n";
        let skill = SKILL.replace("\r\n", "\n");
        let evidence = skill
            .split_once(marker)
            .unwrap()
            .1
            .split("\n## ")
            .next()
            .unwrap();
        for topic in ["audit", "build"] {
            let mcp = read(topic).unwrap();
            let cli = read_cli(topic).unwrap();
            assert_eq!(
                mcp.strip_prefix(MCP_PREAMBLE),
                cli.strip_prefix(CLI_PREAMBLE)
            );
            assert!(mcp.contains(evidence));
            assert!(mcp.contains("Static review is not visual acceptance"));
            assert!(mcp.contains("Relationships") && mcp.contains("Variation"));
            assert!(!mcp.contains("the reference image only confirms them"));
        }
        let audit = read("audit").unwrap();
        assert!(audit.contains("# Audit an existing UI"));
        assert!(audit.contains("review-only request authorizes no application edits"));
        assert!(audit.contains("reuse its existing bundle"));
        assert!(audit.contains("Stop that item"));
        assert!(audit.contains("1, 3 and 7"));
        assert!(audit.contains("workflow:build#building-a-frame"));
        assert!(!audit.contains("](../SKILL.md"));
        assert!(read("build").unwrap().contains("workflow:audit"));
        assert!(INSTRUCTIONS.contains("workflow(topic=audit)"));
    }

    #[test]
    fn build_reports_missing_dependencies_without_blocking_implementation() {
        let build = read("build").unwrap();
        assert!(build.contains("## Font and asset preflight"));
        assert!(build.contains("document.fonts.load"));
        assert!(build.contains("empty array"));
        assert!(build.contains("missing-assets.json"));
        assert!(build.contains("Continue implementation without waiting for missing inputs"));
        assert!(build.contains("final visual acceptance"));
        assert!(!build.contains("Block affected work"));
    }

    #[test]
    fn guides_keep_scope_image_evidence_and_cleanup_boundaries() {
        let build = read("build").unwrap();
        assert!(build.contains("## Building a frame") && build.contains("## Known gaps"));
        assert!(!build.contains("## Setup") && !build.contains("## Component mappings"));
        assert!(build.contains("figlens_bindings") && build.contains("integrations.md"));
        let images = read("images").unwrap();
        assert!(
            images.contains("crop, mask or effects") && images.contains("browser transfer bytes")
        );
        assert!(!images.contains("## Building a frame"));
        // Both guides lead to writes, and an agent may load only one of them.
        assert!(build.contains("## Where outputs go") && images.contains("## Where outputs go"));
        let plan = read("plan").unwrap();
        assert!(plan.contains("A plan-only request ends with the plan"));
        // A whole-scope plan must not shrink to slice 1 and then start implementing.
        assert!(plan.contains("give every slice all four fields and stop with the complete plan"));
        assert!(INSTRUCTIONS.contains("detail every slice, then wait for approval"));
        assert!(plan.contains("(workflow:planning-details#save-and-resume-proportionately)"));
        let details = read("planning-details").unwrap();
        assert!(
            details.contains("Internal Only Canvas")
                && details.contains("Native Dev Mode annotations")
        );
        let yolo = read("yolo").unwrap();
        assert!(yolo.contains("after implementation and the applicable acceptance checks succeed"));
        assert!(
            yolo.contains("unexpected edits") && yolo.contains("leaves the working plan available")
        );
    }
}
