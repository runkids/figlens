# Host plugins

Load this for the per-host plugin manifests at the repository root, which install the shared `skills/figlens/` skill and the figlens MCP server in one step. The npm package's Pi, OpenCode and OMP entries are in [integrations](integrations.md); these manifests do not touch them.

## Layout

The repository root is the plugin root for every host: one `skills/` tree, plus a thin manifest per host. Each MCP entry runs `npx -y @runkids/figlens@<version> mcp`, pinned so the skill and the binary come from one release.

| Host | Files | Skill | MCP |
|---|---|---|---|
| Claude Code | `.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json` | default `skills/` | inline `mcpServers` |
| Codex | `.codex-plugin/plugin.json`, `.codex-plugin/mcp.json`, `.agents/plugins/marketplace.json` | `skills` | `mcpServers` path; each server needs `"type": "stdio"` |
| Antigravity (`agy`) | `.antigravity-plugin/plugin.json`, `mcp_config.json` | default `skills/` | root `mcp_config.json` |
| Cursor | `.cursor-plugin/plugin.json` | `skills` | inline `mcpServers` |
| Kimi Code | `.kimi-plugin/plugin.json` | `skills` | inline `mcpServers` |

No root `.mcp.json`: Claude Code would offer it to everyone who opens this repository. No hooks or session-start injection: the skill is invoked per task and the server serves its own instructions.

## Verification

| Host | Evidence | Not verified |
|---|---|---|
| Claude Code 2.1.316 | `claude plugin validate .`; install from the local marketplace with an isolated `CLAUDE_CONFIG_DIR`; `plugin details` lists skill `figlens` and MCP `figlens`; `claude mcp list` reports it connected | a model session |
| Codex 0.165.0 | local marketplace install with an isolated `CODEX_HOME`; app-server `plugin/read` lists skill `figlens:figlens` and server `figlens`; `mcpServerStatus/list` lists its 17 tools | a model session |
| Antigravity 1.4.2 | `agy plugin validate`: 1 skill, 1 MCP server | install (it writes the real home); a session |
| Cursor, Kimi Code | fields from the hosts' plugin docs and published plugins that ship MCP | no host run |

Codex installs a Git copy of the committed tree: test it after committing, or it reads the previous manifest. Antigravity loads only the first 24,000 bytes of a skill body. The server's `workflow` tool still serves the full guidance.

Not shipped:
- **Hermes Agent.** Its plugin installer clones the whole repository and refuses more than 10 MB or 400 files. Its skill scanner blocks the skill's `curl … | sh` install line, even with `--force`. Users run `hermes mcp add figlens --command npx --args -y @runkids/figlens mcp` instead, which is verified to connect.
- **Muse.** It has no official manifest schema, and its plugins are experimental.

A Git install copies the whole repository, about 50 MB, mostly the demo media in `assets/`.

## Versions

Release Please bumps every manifest `version` and pinned `@runkids/figlens@<version>` argument through `extra-files` in `.github/release-please-config.json`. `scripts/host_plugins_test.py` fails when a manifest version or pinned package differs from `npm/package.json`, when one is missing from `extra-files`, or when a manifest path does not exist.

## Adding a host

1. Read the host's plugin docs or installed CLI for its skill and MCP fields. Do not copy another host's fields.
2. Add its manifest directory, list it in `HOST_FILES`, and add each version field to `extra-files`.
3. Install it with an isolated config directory. Confirm the skill and the server's tools, and record the result in the table above.
4. Update the Plugin setup in `README.md` and every locale.
