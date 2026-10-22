# Reviewed component mappings in Pi

Implemented on 2026-10-23, alongside the local preview release.

- Added the `/figlens` selection, mapping, review, status and close workflow, with two model tools and project-owned Markdown persistence.
- Kept design reading in the existing CLI/MCP and source discovery in the coding agent. Added evidence checks without a Rust mapping resolver or framework-specific parser.
- Included the extension in the existing `@runkids/figlens` npm package through its established CI/Trusted Publisher. The adjacent launcher supplies the matching native binary without a global install.
- Added npm-first onboarding, a workflow diagram, actual viewer/Pi screenshots and a recording using MCP Apps for Claude and the existing plain HTML implementation.
- Verified workflow behavior with Node tests, an installed Pi 1.3.0 loader, scripted dialogs on Aurora/copied Skillshare components, an actual Pi terminal session on MCP Apps, and live/offline preview browser checks. These checks do not measure model quality, generation speed or whole-screen visual accuracy.

Current procedures and limitations live in the `pi`, `preview` and `release` topics.
