const crypto = require('crypto');
const secp256k1 = require('@noble/secp256k1');
const { encryptLauncherForWallet } = require('../src/services/walletLauncher');

const walletPriv = '132a35e500fd693c6c176ce0fdab319616decd89f5a1e54f0eacbbb043c12541';
const walletPub = '034bae003ce515ff11d180093d262e3b403712748226b975a11d8b6250df163c4a';

function hexToBytes(hex) {
  return Uint8Array.from(Buffer.from(hex, 'hex'));
}

function sharedSecretX(privateKey, publicKeyBytes) {
  const compressed = secp256k1.getSharedSecret(privateKey, publicKeyBytes, true);
  return Buffer.from(compressed.slice(1, 33));
}

const encrypted = encryptLauncherForWallet(walletPriv, walletPub);
const shared = sharedSecretX(hexToBytes(walletPriv), hexToBytes(encrypted.ephemeral_public_key));
const aesKey = crypto.createHash('sha256').update(shared).digest('hex');
console.log(JSON.stringify({ aesKey, ephemeral: encrypted.ephemeral_public_key.slice(0, 20) }));
