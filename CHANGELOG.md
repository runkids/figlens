# Changelog

## [0.8.0](https://github.com/runkids/figlens/compare/v0.7.4...v0.8.0) (2026-10-28)


### ⚠ BREAKING CHANGES

* **mcp:** MCP tool calls with fields outside the tool's input schema now fail with `invalid_arguments` instead of being ignored. Use the parameter names from `tools/list`.

### New Features

* **mcp:** reject unknown tool arguments ([2f60ea5](https://github.com/runkids/figlens/commit/2f60ea5737241a8ae73b6985a41e619e8e2aab37))


### Bug Fixes

* **mcp:** keep advertised schema integers JSON-safe ([ceee44a](https://github.com/runkids/figlens/commit/ceee44ace3979dda4418dba009d661e88008c364))

## [0.7.4](https://github.com/runkids/figlens/compare/v0.7.3...v0.7.4) (2026-10-27)


### New Features

* **handoff:** document offline UI case and dependency preflight ([#40](https://github.com/runkids/figlens/issues/40)) ([6e5fac2](https://github.com/runkids/figlens/commit/6e5fac282707f69f59f5c46f52d4ffeb723cafb4))


### Bug Fixes

* **render:** omit SVG group identifiers and provenance ([afe3067](https://github.com/runkids/figlens/commit/afe30670136029c3733b46655c7143ff68ded2ce))
* **render:** scope SVG ids to the exported node ([321e8e6](https://github.com/runkids/figlens/commit/321e8e6188e5d3bf2955737ef70175441086e4a2))
* **render:** scope SVG ids to the exported node ([10609e4](https://github.com/runkids/figlens/commit/10609e43ec6865a6edf28fd4a01c214c7fa0a1ab))

## [0.7.3](https://github.com/runkids/figlens/compare/v0.7.2...v0.7.3) (2026-10-27)


### Bug Fixes

* handle subcommand help and bound original image reads ([ee79f78](https://github.com/runkids/figlens/commit/ee79f78c962b7224c3e40eaf3a9050df20c6fd86))
* protect outputs and bound MCP handoffs ([e12a3d0](https://github.com/runkids/figlens/commit/e12a3d0ed37dc537bb71df8de137d02bd8549075))
* protect outputs and bound MCP handoffs ([dd315dd](https://github.com/runkids/figlens/commit/dd315dd2f0cae966f905c14bb964808902cbfc7d))

## [0.7.2](https://github.com/runkids/figlens/compare/v0.7.1...v0.7.2) (2026-10-26)


### New Features

* compare stored component variants and apply text properties ([61614f8](https://github.com/runkids/figlens/commit/61614f8b8c9b79e4a0f3103430f8400fc7bd932d))
* expose component state evidence and variant comparisons ([f875c0d](https://github.com/runkids/figlens/commit/f875c0df29089e1011c30c6e63e660c95a4608e5))
* expose component variants and prototype evidence ([7ed5310](https://github.com/runkids/figlens/commit/7ed5310f1f5fbfa557a780ab1d7516a8e0c77671))
* expose partial layout semantics and render diagnostics ([eed74be](https://github.com/runkids/figlens/commit/eed74bed7b23b76b41098a5f74c0a96a621a1200))
* expose partial layout semantics and render diagnostics ([6702292](https://github.com/runkids/figlens/commit/67022926b8a8c6acefc8a6f3e17cb887721442ef))


### Bug Fixes

* compare stored left and top padding fields ([9fe6dc1](https://github.com/runkids/figlens/commit/9fe6dc118098721fbe113980a25952035aa504b6))
* preserve text metadata and empty prototype status rows ([b1bf5b0](https://github.com/runkids/figlens/commit/b1bf5b0e6778cb160e902b1811f2ae3db72e29b8))
* reject malformed component parent memberships ([a072da8](https://github.com/runkids/figlens/commit/a072da8b07d764b7c0d77be078acdd86810e1dfd))
* reject malformed GUIDs in shared reference lookups ([e6ea189](https://github.com/runkids/figlens/commit/e6ea1898ba79bdd41a2ecbc29dfe0c357e78e5a4))


### Performance

* **render:** defer stroke geometry checks for diagnostics ([b87cafe](https://github.com/runkids/figlens/commit/b87cafec9f25fe1cf60c39852b433524aa7b1e5f))
* **render:** skip unused pure-blur intermediates ([eb01fe3](https://github.com/runkids/figlens/commit/eb01fe35deb84a829b27a1197e2b123dd05d3121))
* **render:** skip unused pure-blur intermediates ([798fe20](https://github.com/runkids/figlens/commit/798fe20343a790a44b53ceb8090b0e4e924af808))
* skip unused parent definition preparation ([e96b907](https://github.com/runkids/figlens/commit/e96b9073b995450740add80061e89fb17b9d73c1))

## [0.7.1](https://github.com/runkids/figlens/compare/v0.7.0...v0.7.1) (2026-10-25)


### Performance

* avoid full-file scans when opening fig archives ([26cc0a9](https://github.com/runkids/figlens/commit/26cc0a949246b84951e5d63fec28195dc5d775a8))
* avoid full-file scans when opening fig archives ([e93212d](https://github.com/runkids/figlens/commit/e93212db3bcb7cb276eef341f7615de37d023414))
* bound frames only with --jobs and release message bytes after decode ([0e10d14](https://github.com/runkids/figlens/commit/0e10d142726fad08ec82ca395f1116b19542915f))
* decouple frame jobs and release decoded message bytes ([eec2968](https://github.com/runkids/figlens/commit/eec29685aee79b4a218634d67699ee26d298fbb8))
* store decoded Kiwi byte arrays compactly ([4ec0f36](https://github.com/runkids/figlens/commit/4ec0f368825a19a6d2191744b4897cf11df0b3df))
* store decoded Kiwi byte arrays compactly ([2ab7ad5](https://github.com/runkids/figlens/commit/2ab7ad5250a89a23c061253e00248c8bac2ec4fa))

## [0.7.0](https://github.com/runkids/figlens/compare/v0.6.0...v0.7.0) (2026-10-25)


### ⚠ BREAKING CHANGES

* **mcp:** MCP no longer accepts legacy CLI argument coercions, numeric strings or optional null/false sentinels. Use JSON numbers and maxSize: 0 for uncapped exports.

* **mcp:** centralize tool contracts and request admission ([1309faa](https://github.com/runkids/figlens/commit/1309faa1b13eefcc239ebfbd1e1fecaa25e6c520))


### New Features

* expose built-in MCP workflow guidance ([8de61d8](https://github.com/runkids/figlens/commit/8de61d89e60b53e83a081d38c03675124e6fed3e))


### Bug Fixes

* align MCP contracts and clean up smoke processes ([7dcbe8b](https://github.com/runkids/figlens/commit/7dcbe8b48507a9526ffa40716cf5cc05f974369e))
* bound document parsing before Kiwi decoding ([b6e959f](https://github.com/runkids/figlens/commit/b6e959f44148f62f74fdc4175b46a5dc27089a58))
* create candidate output before container builds ([bb45fed](https://github.com/runkids/figlens/commit/bb45fed6af1e5d446b04ccab13520565fbfc28e5))
* harden design parsing and verify installed workflows ([3c482e8](https://github.com/runkids/figlens/commit/3c482e85918d5cdf661bb0e6b2fd32a0769bbb1d))
* resolve instance sublayer IDs consistently ([b9654ae](https://github.com/runkids/figlens/commit/b9654aea4ec618324337d05f64dc70dd748ac2af))
* use Windows system tar for ZIP extraction ([dd33bd2](https://github.com/runkids/figlens/commit/dd33bd258d8eb90f58ae9456b6138b2d205ba137))


### Reverts

* **skill:** drop sizing-intent guidance from the MCP refactor ([b1c1f9d](https://github.com/runkids/figlens/commit/b1c1f9d3092360a22b32d6db5ef408dcc58bfa4c))

## [0.6.0](https://github.com/runkids/figlens/compare/v0.5.0...v0.6.0) (2026-10-24)


### New Features

* add autonomous design implementation workflow ([19d6d08](https://github.com/runkids/figlens/commit/19d6d08d396d14c9a1daa93328f66ca8fb5bf2f0))

## [0.5.0](https://github.com/runkids/figlens/compare/v0.4.2...v0.5.0) (2026-10-24)


### New Features

* add scoped design planning, image evidence and Pi preview ([#16](https://github.com/runkids/figlens/issues/16)) ([36ecf0b](https://github.com/runkids/figlens/commit/36ecf0b44e3b703550597e6cd6f67076fa8964df))

## [0.4.2](https://github.com/runkids/figlens/compare/v0.4.1...v0.4.2) (2026-10-23)


### Bug Fixes

* **agent:** prepare website images using project conventions ([#14](https://github.com/runkids/figlens/issues/14)) ([d5569e8](https://github.com/runkids/figlens/commit/d5569e8786376f5fc57af8fbd074977cb2aad486))

## [0.4.1](https://github.com/runkids/figlens/compare/v0.4.0...v0.4.1) (2026-10-23)


* **skill:** build with the project's components and tokens before raw values ([#12](https://github.com/runkids/figlens/issues/12)) ([961bfbb](https://github.com/runkids/figlens/commit/961bfbba385e3008615d656308f2498a8d417757))

## [0.4.0](https://github.com/runkids/figlens/compare/v0.3.3...v0.4.0) (2026-10-23)


### New Features

* export maxSize, --root folders and review fixes ([edf8a83](https://github.com/runkids/figlens/commit/edf8a833a6fd3137b701cf13e9e4d6d2e0d14de7))
* **export:** cap the longest side with maxSize, 1600 px by default over MCP ([9cdf2be](https://github.com/runkids/figlens/commit/9cdf2be4fb8a9fb759f9854e6917e768c464214c))
* **mcp:** confine files and outputs to --root folders ([3b52502](https://github.com/runkids/figlens/commit/3b52502a75e0c7611f699257d71272fbc919a381))
* **mcp:** keep the parsed file across calls on the same file ([10faa13](https://github.com/runkids/figlens/commit/10faa130aef8a928c1ad73e7c790195f318c2130))
* **mcp:** keep the parsed file across calls on the same file ([93cbf70](https://github.com/runkids/figlens/commit/93cbf7024526d733b884528a2ac7d65c5aecc8a8))


### Bug Fixes

* **mcp:** close three --root and tree-reuse gaps found in review ([a4e6077](https://github.com/runkids/figlens/commit/a4e6077ea72b8ce69002ebbeee20e84cf42f6aa1))
* **mcp:** keep review findings out of the parse reuse and export cap ([948595c](https://github.com/runkids/figlens/commit/948595c9a0b7f11b1d74859e02b1839fce221c92))
* **mcp:** never preload the file of a tools/call notification ([d76270d](https://github.com/runkids/figlens/commit/d76270d60c0d3d5c9e55d450ed23e3bb455c7b5c))

## [0.3.3](https://github.com/runkids/figlens/compare/v0.3.2...v0.3.3) (2026-10-23)


### Bug Fixes

* **render:** preserve offset image fills in clipped exports ([c90a355](https://github.com/runkids/figlens/commit/c90a355c17d943b900d25aceeb810b1a475ef12c))

## [0.3.2](https://github.com/runkids/figlens/compare/v0.3.1...v0.3.2) (2026-10-23)


### New Features

* **export:** support direct SVG output ([0f91f42](https://github.com/runkids/figlens/commit/0f91f421b381cfc39a3168c85169517f3fd4a175))

## [0.3.1](https://github.com/runkids/figlens/compare/v0.3.0...v0.3.1) (2026-10-23)


### Bug Fixes

* **pi:** make local design workflows agent-first ([0b22ffe](https://github.com/runkids/figlens/commit/0b22ffed66231d4331dde8ab93770cc66d4efd10))

## [0.3.0](https://github.com/runkids/figlens/compare/v0.2.0...v0.3.0) (2026-10-22)


### New Features

* add agent discovery and inline MCP images ([609ec85](https://github.com/runkids/figlens/commit/609ec85387e8735c34f4ee9e20daf437f8ef9e20))
* **pi:** reuse reviewed project components from local designs ([8ffcc85](https://github.com/runkids/figlens/commit/8ffcc858359fa148d726985c797ae1667de2bee0))
* **preview:** add a Figma-style local design viewer ([b36384a](https://github.com/runkids/figlens/commit/b36384a90d85a1a5596bebe36f0ba19fc9799e31))
* **preview:** browse a component's variants from the inspector ([d3c8f29](https://github.com/runkids/figlens/commit/d3c8f294714069a034f1e2f7a45b5cca6b9c8163))
* **preview:** export the selection as SVG ([eb49d8d](https://github.com/runkids/figlens/commit/eb49d8d2ae768d22fca2ce12e21c21a93457f413))
* **preview:** pick rotated layers, ellipses and lines by their shape ([6ade124](https://github.com/runkids/figlens/commit/6ade124529c9c398ae1a57d938d697c2a888206d))
* **preview:** pick vector layers by their outlines ([195978b](https://github.com/runkids/figlens/commit/195978be951087b9ef2238dc78bb9010b7352dd9))
* **preview:** select several layers with Shift-click ([b7c2eca](https://github.com/runkids/figlens/commit/b7c2ecae9e6acf68e517fb24ca7bacae8f22aa0d))
* **preview:** show prototype interactions in the inspector ([a5111a9](https://github.com/runkids/figlens/commit/a5111a9feb0891254f22249b351d11079b2fe1d4))


### Bug Fixes

* **npm:** preserve the binary during concurrent first use ([5e87824](https://github.com/runkids/figlens/commit/5e8782411e7b76398a4e6c27d68a0678a149bbad))

## [0.2.0](https://github.com/runkids/figlens/compare/v0.1.2...v0.2.0) (2026-10-21)


### ⚠ BREAKING CHANGES

* export prints a JSON object instead of "<path>: WxH, N KB, quality Q"; read width, height, bytes and quality from it. bundle prints a JSON object on stdout and no longer repeats its warnings on stderr.

### New Features

* print export and bundle results as JSON ([42b15c3](https://github.com/runkids/figlens/commit/42b15c36cab89be736c656409456fb9915afb16c))

## [0.1.2](https://github.com/runkids/figlens/compare/v0.1.1...v0.1.2) (2026-10-21)


* publish to npm through OIDC trusted publishing ([45b705b](https://github.com/runkids/figlens/commit/45b705bcffe4409011663cbe2451fa90d01b2125))

## [0.1.1](https://github.com/runkids/figlens/compare/v0.1.0...v0.1.1) (2026-10-21)


### Bug Fixes

* publish the npm wrapper as @runkids/figlens ([f46333b](https://github.com/runkids/figlens/commit/f46333b323d7748a95e9333c650c6c0da0725425))

## 0.1.0 (2026-10-21)


### New Features

* read Figma .fig files offline ([e77dbd6](https://github.com/runkids/figlens/commit/e77dbd63e3490405a0f7a4a7895a13c689d90ced))
* serve figlens tools over MCP ([c411c2c](https://github.com/runkids/figlens/commit/c411c2c70fda54017e31c8941bf262f3d069c128))


### Bug Fixes

* harden reading crafted .fig files ([2ccc446](https://github.com/runkids/figlens/commit/2ccc446830c0a7aec0d9461468f6850b411d3023))
