# Changelog

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
