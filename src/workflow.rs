//! MCP access to the same workflow sources shipped to Pi and skill users.

const SKILL: &str = include_str!("../skills/figlens/SKILL.md");
pub const TOPICS: &[&str] = &["build", "images", "plan", "planning-details", "yolo"];

pub const INSTRUCTIONS: &str = "Read local Figma .fig snapshots with figlens. Resolve the requested screen from names or visible text using find/tree; ask about recognizable choices only when materially ambiguous, and resolve node IDs yourself. Use inspect for compact layout/text, nodes for full styles/masks/component data, variables for tokens, and export for reference pixels or assets. For implementation, call workflow(topic=build); for implementation planning or careful reproduction, also load plan; for website image preparation, load images. Load planning-details only for page/multi-page scope or unresolved dependencies. Load yolo only when the user explicitly requests autonomous implementation and temporary-plan cleanup. A plan-only request ends with a plan; an authorized build continues through implementation, visual checks and corrections using the project's existing architecture. Simple questions and exports use the design tools directly. Workflow reads return guidance, not executed work; host tools and permissions govern project edits, browser checks and cleanup. Check isError and content on failed calls. Native Dev Mode annotations and font files are unavailable; report material gaps rather than guessing.";

pub fn read(topic: &str) -> Result<String, String> {
    let headings: &[&str] = match topic {
        "build" => &[
            "Where outputs go",
            "Building a frame",
            "Font and asset preflight",
            "Link back to Figma",
            "Reading the output",
            "Known gaps",
        ],
        "images" => &["Where outputs go", "Images and slices", "Known gaps"],
        "plan" | "planning-details" | "yolo" => &[],
        _ => {
            return Err(format!(
                "unknown workflow topic {topic:?}; choose {}",
                TOPICS.join(", ")
            ));
        }
    };
    let mut text = String::from(
        "Use this guidance in the current agent context and within the user's authorized scope. It does not execute work or grant permissions. Use the configured figlens MCP tools with absolute file/output paths; CLI examples describe equivalent tool arguments. Project inspection/edits, directory creation, bundle extraction and browser checks require the host's own tools. Retrieve another topic only for missing guidance needed now: images covers image delivery; build covers design reading and project reuse. Links beginning workflow: name another topic of this tool, with an optional section anchor; no local skill installation is needed.\n\n",
    );
    match topic {
        "plan" => text.push_str(include_str!("../skills/figlens/references/planning.md")),
        "planning-details" => text.push_str(include_str!(
            "../skills/figlens/references/planning-details.md"
        )),
        "yolo" => text.push_str(include_str!("../skills/figlens/references/yolo.md")),
        _ => {
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
        }
    }
    for (file, topic) in [
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
        assert!(!build.contains("## Setup") && !build.contains("## Pi component mappings"));
        let images = read("images").unwrap();
        assert!(
            images.contains("crop, mask or effects") && images.contains("browser transfer bytes")
        );
        assert!(!images.contains("## Building a frame"));
        // Both guides lead to writes, and an agent may load only one of them.
        assert!(build.contains("## Where outputs go") && images.contains("## Where outputs go"));
        let plan = read("plan").unwrap();
        assert!(plan.contains("A plan-only request ends with the plan"));
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
