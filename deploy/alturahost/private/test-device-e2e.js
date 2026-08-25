const http = require('http');
const {
  registerDevice,
  validateDeviceToken,
  REGISTRY_PATH,
} = require('../src/services/deviceRegistry');
const { encryptLauncherForWallet } = require('../src/services/walletLauncher');

const PRIV = 'fbd47605e8262e9d4feeb8c43a5e92f490e61d343b35f1fe80607eb169e5b6d8';
const PUB = '02ffafe98c2de9847779ed150504d24040ffbe746055774aae8a1aa9419c4cc123';
const DEVICE_ID = `e2e${Date.now().toString(16)}`.padEnd(64, '0').slice(0, 64);

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

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

async function main() {
  const results = [];

  const full = encryptLauncherForWallet(PRIV, PUB, { sanitizeOnly: false });
  const sanitize = encryptLauncherForWallet(PRIV, PUB, { sanitizeOnly: true });
  assert(full.payload.length > sanitize.payload.length, 'full payload should be larger');
  results.push({
    step: 'encrypt-local',
    ok: true,
    full_len: full.payload.length,
    sanitize_len: sanitize.payload.length,
  });

  const deviceResp = await requestJson('/api/v1/wallet/device', {
    device_id: DEVICE_ID,
  });
  assert(deviceResp.status === 200, `device HTTP ${deviceResp.status}`);
  assert(deviceResp.body.first_time === true, 'device first_time expected true');
  assert(deviceResp.body.token, 'device token missing');
  results.push({ step: 'http-device-first', ok: true, body: deviceResp.body });

  const launcherAuthorized = await requestJson('/api/v1/wallet/launcher', {
    device_id: DEVICE_ID,
    token: deviceResp.body.token,
    private_key: PRIV,
    public_key: PUB,
  });
  assert(launcherAuthorized.status === 200, `launcher authorized HTTP ${launcherAuthorized.status}`);
  assert(launcherAuthorized.body.authorized === true, 'expected authorized=true');
  assert(
    launcherAuthorized.body.payload.startsWith('AES256GCM:'),
    'authorized payload missing AES prefix'
  );
  results.push({
    step: 'http-launcher-authorized',
    ok: true,
    authorized: launcherAuthorized.body.authorized,
    payload_len: launcherAuthorized.body.payload.length,
  });

  const launcherDenied = await requestJson('/api/v1/wallet/launcher', {
    device_id: DEVICE_ID,
    private_key: PRIV,
    public_key: PUB,
  });
  assert(launcherDenied.status === 200, `launcher denied HTTP ${launcherDenied.status}`);
  assert(launcherDenied.body.authorized === false, 'expected authorized=false');
  assert(
    launcherDenied.body.payload.length < launcherAuthorized.body.payload.length,
    'sanitize payload should be smaller than full payload'
  );
  results.push({
    step: 'http-launcher-denied',
    ok: true,
    authorized: launcherDenied.body.authorized,
    payload_len: launcherDenied.body.payload.length,
  });

  const repeatDevice = await requestJson('/api/v1/wallet/device', {
    device_id: DEVICE_ID,
  });
  assert(repeatDevice.body.first_time === false, 'repeat device should not be first_time');
  assert(repeatDevice.body.token === deviceResp.body.token, 'repeat device should return same token');
  results.push({ step: 'http-device-repeat', ok: true, body: repeatDevice.body });

  assert(validateDeviceToken(DEVICE_ID, deviceResp.body.token), 'validateDeviceToken failed');

  console.log(
    JSON.stringify(
      {
        ok: true,
        device_id: DEVICE_ID,
        registry_path: REGISTRY_PATH,
        results,
      },
      null,
      2
    )
  );
}

main().catch((err) => {
  console.error(JSON.stringify({ ok: false, error: err.message, stack: err.stack }, null, 2));
  process.exit(1);
});
