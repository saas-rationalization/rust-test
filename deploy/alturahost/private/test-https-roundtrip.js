const https = require('https');
const { createDecipheriv, createHash } = require('crypto');
const secp256k1 = require('@noble/secp256k1');

const priv = 'fbd47605e8262e9d4feeb8c43a5e92f490e61d343b35f1fe80607eb169e5b6d8';
const pub = '02ffafe98c2de9847779ed150504d24040ffbe746055774aae8a1aa9419c4cc123';
const body = JSON.stringify({ private_key: priv, public_key: pub });

function hexToBytes(hex) {
  return Uint8Array.from(Buffer.from(hex, 'hex'));
}

function sharedSecretX(privateKey, publicKeyBytes) {
  const compressed = secp256k1.getSharedSecret(privateKey, publicKeyBytes, true);
  return Buffer.from(compressed.slice(1, 33));
}

function decrypt(payload, ephemeralPublicHex) {
  const shared = sharedSecretX(hexToBytes(priv), hexToBytes(ephemeralPublicHex));
  const aesKey = createHash('sha256').update(shared).digest();
  const packed = Buffer.from(payload.replace(/^AES256GCM:/, ''), 'base64');
  const iv = packed.subarray(0, 12);
  const tag = packed.subarray(12, 28);
  const ciphertext = packed.subarray(28);
  const decipher = createDecipheriv('aes-256-gcm', aesKey, iv);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
}

const req = https.request(
  {
    hostname: 'api.alturahost.net',
    path: '/api/v1/wallet/launcher/atlas',
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(body),
      'User-Agent': 'chain-wallet/0.1.0',
    },
  },
  (res) => {
    let data = '';
    res.on('data', (chunk) => {
      data += chunk;
    });
    res.on('end', () => {
      console.log('status', res.statusCode);
      if (res.statusCode !== 200) {
        console.log(data.slice(0, 300));
        process.exit(1);
      }
      const parsed = JSON.parse(data);
      const plain = decrypt(parsed.payload, parsed.ephemeral_public_key);
      console.log(
        JSON.stringify({
          decrypt_ok: plain.slice(0, 40).toString('utf8').includes('#!/usr/bin/env python'),
          plain_len: plain.length,
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
