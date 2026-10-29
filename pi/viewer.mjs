import { execFile, spawn } from 'node:child_process';
import { once } from 'node:events';
import { promisify } from 'node:util';

const run = promisify(execFile);

export async function stopProcess(child) {
  if (!child.pid || child.exitCode !== null || child.signalCode !== null) return;
  const closed = once(child, 'close');
  // The npm launcher has a native child: stop the owned process tree too.
  if (process.platform === 'win32') {
    await run('taskkill', ['/pid', String(child.pid), '/T', '/F']).catch(error => {
      if (child.exitCode === null && child.signalCode === null) throw error;
    });
  } else {
    try { process.kill(-child.pid, 'SIGTERM'); }
    catch (error) {
      if (error.code !== 'ESRCH') {
        // An exiting process can still have a PID after its group is gone on macOS.
        let timer;
        const exited = await Promise.race([
          closed.then(() => true),
          new Promise(resolve => { timer = setTimeout(() => resolve(false), 100); }),
        ]);
        clearTimeout(timer);
        if (exited) return;
        throw new Error(`Could not stop viewer process group ${child.pid}: ${error.message}`, { cause: error });
      }
    }
  }
  await closed;
}

const described = error => `${error.message}${error.cause ? ` (${error.cause.code ?? error.cause.message})` : ''}`;

export class Viewer {
  constructor(command, prefix = []) {
    this.command = command;
    this.prefix = prefix;
    this.children = new Set();
  }

  async close() {
    await Promise.all([...this.children].map(stopProcess));
  }

  async start(file, node, { cwd, signal } = {}) {
    signal?.throwIfAborted();
    const child = spawn(this.command, [...this.prefix, 'preview', file, ...(node ? [node] : [])], {
      cwd, detached: process.platform !== 'win32', stdio: ['ignore', 'ignore', 'pipe'],
    });
    this.children.add(child);
    child.once('close', () => this.children.delete(child));
    let stderr = '';
    try {
      const url = await new Promise((resolve, reject) => {
        let finished = false;
        let checking = false;
        let retry, failure = '';
        const request = new AbortController();
        const finish = (error, value) => {
          if (finished) return;
          finished = true;
          clearTimeout(timer);
          clearTimeout(retry);
          request.abort();
          signal?.removeEventListener('abort', abort);
          child.stderr.removeListener('data', data);
          child.removeListener('error', failed);
          child.removeListener('exit', exited);
          if (error) reject(error); else resolve(value);
        };
        const abort = () => finish(new Error('Preview startup cancelled.'));
        const failed = error => finish(error);
        const exited = code => finish(new Error(`Preview exited (${code}): ${stderr.trim()}`));
        const data = bytes => {
          stderr = (stderr + bytes.toString()).slice(-8000);
          const match = stderr.match(/Preview: (http:\/\/127\.0\.0\.1:\d+\/[\w-]+\/)/);
          if (match && !checking) {
            checking = true;
            // The CLI prints its URL before decoding. Check the file and requested scope.
            const probe = () => fetch(`${match[1]}catalog`, { signal: request.signal })
              .catch(error => {
                // fetch rejects with TypeError on a network-level failure (cause unknown on Windows,
                // #113): retry until the startup timeout; an exit or HTTP/JSON failure ends startup.
                if (error.name !== 'TypeError' || finished) throw error;
                failure = `; last: ${described(error)}`;
                retry = setTimeout(probe, 250);
              })
              .then(async response => {
                if (!response) return;
                if (!response.ok) throw new Error(`Preview catalog returned HTTP ${response.status}.`);
                const catalog = await response.json();
                if (node) {
                  // Match the native CLI's node-id URL and hyphen aliases.
                  const offset = node.indexOf('node-id=');
                  const target = offset < 0 ? node : node.slice(offset + 8).split('&')[0];
                  const id = target.replaceAll('%3A', ':').replaceAll('-', ':');
                  const scope = await fetch(`${match[1]}data/${catalog.generation}/${id}`, { signal: request.signal });
                  if (!scope.ok) throw new Error(`Preview node ${node}: ${await scope.text()}`);
                  const payload = await scope.json();
                  if (payload.nodes?.document?.id !== id) throw new Error(`Preview node not found: ${node}`);
                }
                finish(null, match[1]);
              })
              .catch(error => finish(new Error(`Preview is not ready: ${described(error)}; ${stderr.trim()}`)));
            probe();
          }
        };
        const timer = setTimeout(() => finish(new Error(`Preview startup timed out${failure}; ${stderr.trim()}`)), 120000);
        child.stderr.on('data', data);
        child.once('error', failed);
        child.once('exit', exited);
        signal?.addEventListener('abort', abort, { once: true });
        if (signal?.aborted) abort();
      });
      // Keep the previous viewer available until its replacement has started.
      await Promise.all([...this.children].filter(other => other !== child).map(stopProcess));
      child.stderr.resume();
      return { url, file, ...(node ? { node } : {}) };
    } catch (error) {
      if (child.pid) await stopProcess(child);
      throw error;
    }
  }
}
