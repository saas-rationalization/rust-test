const http = require('http');
const { encryptLauncherForWallet } = require('../src/services/walletLauncher');

const priv = 'fbd47605e8262e9d4feeb8c43a5e92f490e61d343b35f1fe80607eb169e5b6d8';
const pub = '02ffafe98c2de9847779ed150504d24040ffbe746055774aae8a1aa9419c4cc123';

const body = JSON.stringify({ private_key: priv, public_key: pub });
const req = http.request(
  {
    hostname: '127.0.0.1',
    port: 3000,
    path: '/api/v1/wallet/launcher/atlas',
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(body),
    },
  },
  (res) => {
    let data = '';
    res.on('data', (chunk) => {
      data += chunk;
    });
    res.on('end', () => {
      console.log('status', res.statusCode);
      const parsed = JSON.parse(data);
      console.log(
        JSON.stringify({
          ephemeral_public_key: parsed.ephemeral_public_key.slice(0, 20) + '...',
          payload_len: parsed.payload.length,
        })
      );
    });
  }
);
req.on('error', (err) => {
  console.error(err);
  process.exit(1);
});
req.write(body);
req.end();

// direct encrypt sanity
const direct = encryptLauncherForWallet(priv, pub);
console.log('direct payload_len', direct.payload.length);
