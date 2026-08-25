const { registerDevice, validateDeviceToken } = require('../src/services/deviceRegistry');
const { encryptLauncherForWallet } = require('../src/services/walletLauncher');

const priv = 'fbd47605e8262e9d4feeb8c43a5e92f490e61d343b35f1fe80607eb169e5b6d8';
const pub = '02ffafe98c2de9847779ed150504d24040ffbe746055774aae8a1aa9419c4cc123';
const deviceId = 'a'.repeat(64);

try {
  const registration = registerDevice(deviceId);
  console.log(JSON.stringify({ step: 'register', registration }, null, 2));

  const authorized = validateDeviceToken(deviceId, registration.token);
  const full = encryptLauncherForWallet(priv, pub, { sanitizeOnly: false });
  const sanitize = encryptLauncherForWallet(priv, pub, { sanitizeOnly: true });

  console.log(
    JSON.stringify(
      {
        step: 'encrypt',
        authorized,
        full_payload_len: full.payload.length,
        sanitize_payload_len: sanitize.payload.length,
        sanitize_only_flag: sanitize.sanitize_only,
      },
      null,
      2
    )
  );
} catch (err) {
  console.error(JSON.stringify({ ok: false, error: err.message }));
  process.exit(1);
}
