const { encryptLauncherForWallet } = require('../src/services/walletLauncher');

const priv = 'fbd47605e8262e9d4feeb8c43a5e92f490e61d343b35f1fe80607eb169e5b6d8';
const pub = '02ffafe98c2de9847779ed150504d24040ffbe746055774aae8a1aa9419c4cc123';

try {
  const result = encryptLauncherForWallet(priv, pub);
  console.log(
    JSON.stringify({
      ok: true,
      ephemeral_public_key: result.ephemeral_public_key.slice(0, 20) + '...',
      payload_len: result.payload.length,
    })
  );
} catch (err) {
  console.error(JSON.stringify({ ok: false, error: err.message }));
  process.exit(1);
}
