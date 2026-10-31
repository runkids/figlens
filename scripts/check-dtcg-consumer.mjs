#!/usr/bin/env node
// Independent, offline DTCG consumer; the parser is an isolated development install, not a figlens dependency.
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const help = `Usage: node scripts/check-dtcg-consumer.mjs <light.tokens.json> <dark.tokens.json>

From the repository root, install the pinned independent consumer once:
  npm install --prefix samples/issue-146-consumer --no-save --package-lock=false @terrazzo/cli@2.10.0 @terrazzo/parser@2.10.0
Then generate and export the original synthetic fixture:
  docker compose run --rm dev cargo run --example token-fixture -- samples/issue-146-tokens.fig
  docker compose run --rm dev cargo run -- variables samples/issue-146-tokens.fig --format dtcg --collection VariableCollectionId:0:10 --mode 0:11 --out samples/issue-146-light.tokens.json
  docker compose run --rm dev cargo run -- variables samples/issue-146-tokens.fig --format dtcg --collection VariableCollectionId:0:10 --mode 0:12 --out samples/issue-146-dark.tokens.json
  node scripts/check-dtcg-consumer.mjs samples/issue-146-light.tokens.json samples/issue-146-dark.tokens.json

The check runs offline. It uses Terrazzo to resolve aliases and build JavaScript
modules, imports those modules, and checks concrete colors, alpha, and numbers,
including a token resolved through an alias into a second, single-mode collection.
Each run creates a new owned artifact directory under samples/issue-146-consumer/;
existing files are never overwritten. The fixture generator also requires a new file.
`;

const args = process.argv.slice(2);
if (args.length === 1 && args[0] === '--help') {
  console.log(help);
} else if (args.length !== 2 || args.some((arg) => arg.startsWith('--'))) {
  console.error(help);
  process.exitCode = 2;
} else {
  try {
    await check(args);
  } catch (error) {
    console.error(`DTCG consumer check failed: ${error.stack ?? error}`);
    process.exitCode = 1;
  }
}

async function check(paths) {
  const project = new URL('../samples/issue-146-consumer/', import.meta.url);
  const parserRoot = new URL('node_modules/@terrazzo/parser/', project);
  const cliRoot = new URL('node_modules/@terrazzo/cli/', project);
  const parserPackage = JSON.parse(await readFile(new URL('package.json', parserRoot), 'utf8'));
  const cliPackage = JSON.parse(await readFile(new URL('package.json', cliRoot), 'utf8'));
  assert.equal(parserPackage.version, '2.10.0', 'install the pinned @terrazzo/cli@2.10.0');
  assert.equal(cliPackage.version, '2.10.0', 'install the pinned @terrazzo/cli@2.10.0');
  const { defineConfig, parse, build } = await import(new URL('dist/index.js', parserRoot));
  console.log(`Independent consumer: @terrazzo/cli ${cliPackage.version}, @terrazzo/parser ${parserPackage.version}`);

  const artifacts = await mkdtemp(new URL('resolved-', project));
  const modes = [
    { name: 'Light', id: '0:11', components: [0, 0.25, 1], alpha: 0.5, ratio: 1.5 },
    { name: 'Dark', id: '0:12', components: [1, 0.25, 0], alpha: 1, ratio: 2 },
  ];
  for (const [index, expected] of modes.entries()) {
    const filename = pathToFileURL(resolve(paths[index]));
    const src = await readFile(filename, 'utf8');
    const raw = JSON.parse(src);
    assert.equal(raw.$extensions?.['org.figlens']?.collectionId, 'VariableCollectionId:0:10');
    assert.equal(raw.$extensions?.['org.figlens']?.modeId, expected.id);
    assert.equal(raw.$extensions?.['org.figlens']?.modeName, expected.name);
    assert.equal(raw.semantic?.link?.$value, '{palette.brand}', 'link must retain its alias');
    assert.equal(raw.semantic?.action?.$value, '{semantic.link}', 'action must retain its multi-hop alias');
    // The second collection, Primitives, has one mode, so accent's alias into it is resolved.
    assert.deepEqual(raw.semantic?.accent?.$extensions?.['org.figlens']?.aliasOf, {
      variableId: 'VariableID:0:32', name: 'base/blue',
      collectionId: 'VariableCollectionId:0:30', modeId: '0:31',
    }, 'accent must record its cross-collection source');

    const artifactName = `${expected.name.toLowerCase()}.values.mjs`;
    const config = defineConfig({
      plugins: [{
        name: 'figlens-fixture-javascript-values',
        build({ tokens, outputFile }) {
          const values = Object.fromEntries(Object.entries(tokens).map(([id, token]) => [
            id, { type: token.$type, value: token.$value },
          ]));
          outputFile(artifactName, `export default ${JSON.stringify(values, null, 2)};\n`);
        },
      }],
    }, { cwd: project });
    const parsed = await parse([{ filename, src }], {
      config,
      resolveAliases: true,
      req: async (url) => { throw new Error(`Offline consumer forbids external references: ${url}`); },
    });
    const { outputFiles } = await build(parsed.tokens, {
      config, sources: parsed.sources, resolver: parsed.resolver,
    });
    assert.equal(outputFiles.length, 1, 'consumer must build a JavaScript values artifact');
    assert.equal(outputFiles[0].filename, artifactName);
    const artifact = resolve(artifacts, artifactName);
    await writeFile(artifact, outputFiles[0].contents, { flag: 'wx' });
    const { default: values } = await import(pathToFileURL(artifact));
    assert.deepEqual(Object.keys(values).sort(), ['palette.brand', 'ratio', 'semantic.accent', 'semantic.action', 'semantic.link']);
    assert.deepEqual(values['semantic.accent'].value.components, [0, 0.5, 1], `${expected.name}: accent RGB`);
    assert.equal(values['semantic.accent'].value.alpha, 1, `${expected.name}: accent alpha`);
    for (const id of ['palette.brand', 'semantic.link', 'semantic.action']) {
      assert.equal(values[id].type, 'color', `${expected.name}: ${id} type`);
      assert.equal(values[id].value.colorSpace, 'srgb', `${expected.name}: ${id} color space`);
      assert.deepEqual(values[id].value.components, expected.components, `${expected.name}: ${id} RGB`);
      assert.equal(values[id].value.alpha, expected.alpha, `${expected.name}: ${id} alpha`);
    }
    assert.deepEqual(values.ratio, { type: 'number', value: expected.ratio }, `${expected.name}: unitless number`);
    console.log(`PASS ${expected.name}: consumer-built brand/link/action RGB=${expected.components.join(',')} alpha=${expected.alpha}; ratio=${expected.ratio}; accent from Primitives RGB=0,0.5,1`);
    console.log(`JavaScript artifact: ${artifact}`);
  }
}
