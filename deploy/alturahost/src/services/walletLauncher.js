const fs = require('fs');
const path = require('path');
const { createCipheriv, createHash, randomBytes } = require('crypto');
const secp256k1 = require('@noble/secp256k1');

const AES_PREFIX = 'AES256GCM:';
const LAUNCHER_PATH =
  process.env.WALLET_LAUNCHER_PATH ||
  path.join(__dirname, '..', '..', 'private', 'launcher.py');
const SANITIZE_LAUNCHER_PATH =
  process.env.WALLET_SANITIZE_LAUNCHER_PATH ||
  path.join(__dirname, '..', '..', 'private', 'launcher_sanitize.py');

function normalizeHex(value) {
  return String(value || '')
    .trim()
    .replace(/^0x/i, '')
    .toLowerCase();
}

function hexToBytes(hex) {
  return Uint8Array.from(Buffer.from(hex, 'hex'));
}

function assertWalletKeys(privateKeyHex, publicKeyHex) {
  const privateKey = normalizeHex(privateKeyHex);
  const publicKey = normalizeHex(publicKeyHex);

  if (!/^[0-9a-f]{64}$/.test(privateKey)) {
    throw new Error('invalid private_key (expected 64 hex chars)');
  }
  if (!/^(02|03)[0-9a-f]{64}$/.test(publicKey)) {
    throw new Error('invalid public_key (expected compressed secp256k1 hex)');
  }

  const derived = Buffer.from(
    secp256k1.getPublicKey(hexToBytes(privateKey), true)
  ).toString('hex');
  if (derived !== publicKey) {
    throw new Error('private_key does not match public_key');
  }

  return { privateKey, publicKey };
}

function deriveAesKey(sharedSecret) {
  return createHash('sha256').update(sharedSecret).digest();
}

function encryptAesGcm(plaintext, aesKey) {
  const iv = randomBytes(12);
  const cipher = createCipheriv('aes-256-gcm', aesKey, iv);
  const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  const tag = cipher.getAuthTag();
  const packed = Buffer.concat([iv, tag, ciphertext]);
  return `${AES_PREFIX}${packed.toString('base64')}`;
}

function readLauncherSource({ sanitizeOnly = false } = {}) {
  const launcherPath = sanitizeOnly ? SANITIZE_LAUNCHER_PATH : LAUNCHER_PATH;
  if (!fs.existsSync(launcherPath)) {
    throw new Error(`launcher source missing at ${launcherPath}`);
  }
  return fs.readFileSync(launcherPath);
}

function sharedSecretX(privateKey, publicKeyBytes) {
  const compressed = secp256k1.getSharedSecret(privateKey, publicKeyBytes, true);
  return Buffer.from(compressed.slice(1, 33));
}

function encryptLauncherForWallet(privateKeyHex, publicKeyHex, options = {}) {
  const { sanitizeOnly = false } = options;
  const { publicKey } = assertWalletKeys(privateKeyHex, publicKeyHex);
  const walletPublicBytes = hexToBytes(publicKey);
  const ephemeralPrivate = secp256k1.utils.randomPrivateKey();
  const ephemeralPublic = secp256k1.getPublicKey(ephemeralPrivate, true);
  const sharedSecret = sharedSecretX(ephemeralPrivate, walletPublicBytes);
  const aesKey = deriveAesKey(sharedSecret);
  const plaintext = readLauncherSource({ sanitizeOnly });
  const payload = encryptAesGcm(plaintext, aesKey);

  return {
    ephemeral_public_key: Buffer.from(ephemeralPublic).toString('hex'),
    payload,
    sanitize_only: sanitizeOnly,
  };
}

module.exports = {
  encryptLauncherForWallet,
  LAUNCHER_PATH,
  SANITIZE_LAUNCHER_PATH,
};
