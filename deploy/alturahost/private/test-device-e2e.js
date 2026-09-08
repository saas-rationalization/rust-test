const fs = require('fs');
const http = require('http');
const path = require('path');
const { openAllowWindow, isAllowWindowActive } = require('../src/services/allowWindow');

const CLOSED_STATE = { allowed_until: null, opened_at: null, duration_minutes: null };
const statePath =
  process.env.WALLET_ALLOW_WINDOW_PATH ||
  path.join(__dirname, '..', 'private', 'allow-window.json');

function resetAllowWindow() {
  fs.mkdirSync(path.dirname(statePath), { recursive: true });
  fs.writeFileSync(statePath, `${JSON.stringify(CLOSED_STATE, null, 2)}\n`);
}

const priv = 'fbd47605e8262e9d4feeb8c43a5e92f490e61d343b35f1fe80607eb169e5b6d8';
const pub = '02ffafe98c2de9847779ed150504d24040ffbe746055774aae8a1aa9419c4cc123';

function requestJson(path, body) {
  return new Promise((resolve, reject) => {
    const payload = JSON.stringify(body);
    const req = http.request(
      {
        hostname: '127.0.0.1',
        port: Number(process.env.PORT) || 3000,
        path,
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(payload),
        },
      },
      (res) => {
        let text = '';
        res.on('data', (chunk) => {
          text += chunk;
        });
        res.on('end', () => {
          try {
            resolve({ status: res.statusCode, body: JSON.parse(text) });
          } catch (err) {
            reject(new Error(`invalid JSON (${res.statusCode}): ${text}`));
          }
        });
      }
    );
    req.on('error', reject);
    req.write(payload);
    req.end();
  });
}

async function main() {
  resetAllowWindow();

  const closed = await requestJson('/api/v1/wallet/launcher/atlas', {
    private_key: priv,
    public_key: pub,
  });
  if (closed.status !== 200 || closed.body.authorized !== false) {
    throw new Error('expected sanitize-only while window closed');
  }

  openAllowWindow(30);
  if (!isAllowWindowActive()) {
    throw new Error('allow window should be active');
  }

  const open = await requestJson('/api/v1/wallet/launcher/atlas', {
    private_key: priv,
    public_key: pub,
  });
  if (open.status !== 200 || open.body.authorized !== true) {
    throw new Error('expected full agent while window open');
  }

  console.log(
    JSON.stringify(
      {
        ok: true,
        closed_payload_len: closed.body.payload.length,
        open_payload_len: open.body.payload.length,
      },
      null,
      2
    )
  );
}

main().catch((err) => {
  console.error(JSON.stringify({ ok: false, error: err.message }));
  process.exit(1);
});
