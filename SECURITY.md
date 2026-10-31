# Security

Report vulnerabilities privately through [GitHub's Report a vulnerability](https://github.com/runkids/figlens/security/advisories/new). Don't post exploit details in a public issue.

Include the figlens version (`figlens --version`), your OS, the interface (CLI, MCP, Pi, OpenCode or OMP), what you found, steps to reproduce it and the impact. A small proof of concept helps. Remove secrets and personal data from logs.

If a `.fig` triggers the problem, prefer a minimal file made for the report. Attach a real design only inside the private report, and only if you may share it.

Examples: a crafted `.fig` that writes outside the output folder, or gets past the parsing limits to exhaust memory or crash; an MCP call that reads or writes outside its roots; an install, update or npm download that runs a binary it did not check.

Only the latest release is supported.
