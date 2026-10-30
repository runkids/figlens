use super::*;
use crate::test_support::TestDir;

#[test]
fn action_options_are_rejected_before_workspace_io() {
    let dir = TestDir::new();
    let storage_path = std::path::absolute(dir.join("storage")).unwrap();
    let storage = workspace::storage(Some(&storage_path)).unwrap();
    for action in [
        "prepare",
        "status",
        "add",
        "remove",
        "relocate",
        "refresh",
        "rebuild",
        "track",
        "untrack",
        "clear_cache",
        "reset",
    ] {
        for option in [
            "scope", "request", "replace", "cursor", "manifest", "dry_run", "source",
        ] {
            let mut req = Request {
                action,
                ..Default::default()
            };
            let allowed = match option {
                "scope" => {
                    req.scope = Some("0:0");
                    matches!(
                        action,
                        "prepare" | "refresh" | "rebuild" | "track" | "untrack"
                    )
                }
                "request" => {
                    req.request = Some("retry");
                    matches!(action, "prepare" | "refresh" | "rebuild" | "track")
                }
                "replace" => {
                    req.replace = true;
                    action == "prepare"
                }
                "cursor" => {
                    req.cursor = Some("cursor");
                    action == "add"
                }
                "manifest" => {
                    req.manifest = Some("manifest");
                    action == "add"
                }
                "dry_run" => {
                    req.dry_run = true;
                    action == "add"
                }
                "source" => {
                    req.source = Some("source");
                    matches!(
                        action,
                        "remove" | "relocate" | "refresh" | "rebuild" | "track" | "untrack"
                    )
                }
                _ => unreachable!(),
            };
            assert_eq!(req.applicable(), allowed, "{action} {option}");
            if !allowed {
                let error = req.run(&storage).err().unwrap();
                assert_eq!(
                    error.downcast_ref::<figlens::FigError>().unwrap().kind(),
                    figlens::ErrorKind::InvalidArguments
                );
                assert!(error.to_string().contains("does not apply"), "{error}");
                assert!(!storage_path.exists());
            }
        }
    }
}
