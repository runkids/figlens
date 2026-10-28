const { spawn } = require('node:child_process');
const path = require('node:path');

const override = process.env.FIGLENS_BIN;
const child = spawn(override || process.execPath,
  [...(override ? [] : [path.join(__dirname, '../bin/figlens.js')]), ...process.argv.slice(2)],
  { stdio: 'inherit' });
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
child.on('error', error => { console.error(error.message); process.exitCode = 1; });
child.on('exit', (code, signal) => {
  if (signal) {
    process.removeAllListeners(signal);
    process.kill(process.pid, signal);
  } else process.exitCode = code;
});
