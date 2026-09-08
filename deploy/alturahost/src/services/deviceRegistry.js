const fs = require('fs');
const path = require('path');
const { randomBytes } = require('crypto');

const REGISTRY_PATH =
  process.env.WALLET_DEVICE_REGISTRY_PATH ||
  path.join(__dirname, '..', '..', 'private', 'device-registry.json');

const TOKEN_TTL_MS = Number(process.env.WALLET_DEVICE_TOKEN_TTL_MS) || 30 * 60 * 1000;

function normalizeDeviceId(value) {
  const deviceId = String(value || '').trim().toLowerCase();
  if (!/^[0-9a-f]{16,128}$/.test(deviceId)) {
    throw new Error('invalid device_id (expected 16-128 lowercase hex chars)');
  }
  return deviceId;
}

function loadRegistry() {
  if (!fs.existsSync(REGISTRY_PATH)) {
    return { devices: {} };
  }

  try {
    const parsed = JSON.parse(fs.readFileSync(REGISTRY_PATH, 'utf8'));
    if (!parsed || typeof parsed !== 'object' || typeof parsed.devices !== 'object') {
      return { devices: {} };
    }
    return parsed;
  } catch (_err) {
    return { devices: {} };
  }
}

function saveRegistry(registry) {
  const dir = path.dirname(REGISTRY_PATH);
  fs.mkdirSync(dir, { recursive: true });
  const tmpPath = `${REGISTRY_PATH}.${process.pid}.${Date.now()}.tmp`;
  fs.writeFileSync(tmpPath, `${JSON.stringify(registry, null, 2)}\n`, 'utf8');
  fs.renameSync(tmpPath, REGISTRY_PATH);
}

function nowIso() {
  return new Date().toISOString();
}

function isTokenActive(entry, nowMs = Date.now()) {
  if (!entry || !entry.token || !entry.expires_at) {
    return false;
  }
  const expiresMs = Date.parse(entry.expires_at);
  return Number.isFinite(expiresMs) && expiresMs > nowMs;
}

function activeTokenRecord(deviceRecord, nowMs = Date.now()) {
  const tokens = Array.isArray(deviceRecord?.tokens) ? deviceRecord.tokens : [];
  for (let index = tokens.length - 1; index >= 0; index -= 1) {
    const entry = tokens[index];
    if (isTokenActive(entry, nowMs)) {
      return entry;
    }
  }
  return null;
}

function issueToken(deviceId) {
  const token = randomBytes(32).toString('hex');
  const issuedAt = nowIso();
  const expiresAt = new Date(Date.now() + TOKEN_TTL_MS).toISOString();
  return {
    token,
    issued_at: issuedAt,
    expires_at: expiresAt,
    device_id: deviceId,
  };
}

function registerDevice(deviceIdInput) {
  const deviceId = normalizeDeviceId(deviceIdInput);
  const registry = loadRegistry();
  const existing = registry.devices[deviceId];
  const nowMs = Date.now();

  if (existing) {
    const active = activeTokenRecord(existing, nowMs);
    if (active) {
      return {
        device_id: deviceId,
        token: active.token,
        expires_at: active.expires_at,
        first_time: false,
        authorized: true,
      };
    }

    return {
      device_id: deviceId,
      token: null,
      expires_at: null,
      first_time: false,
      authorized: false,
    };
  }

  const tokenRecord = issueToken(deviceId);
  registry.devices[deviceId] = {
    first_seen: nowIso(),
    tokens: [tokenRecord],
  };
  saveRegistry(registry);

  return {
    device_id: deviceId,
    token: tokenRecord.token,
    expires_at: tokenRecord.expires_at,
    first_time: true,
    authorized: true,
  };
}

function validateDeviceToken(deviceIdInput, tokenInput) {
  const deviceId = normalizeDeviceId(deviceIdInput);
  const token = String(tokenInput || '').trim().toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(token)) {
    return false;
  }

  const registry = loadRegistry();
  const deviceRecord = registry.devices[deviceId];
  if (!deviceRecord) {
    return false;
  }

  const nowMs = Date.now();
  const tokens = Array.isArray(deviceRecord.tokens) ? deviceRecord.tokens : [];
  return tokens.some(
    (entry) =>
      entry &&
      String(entry.token || '').toLowerCase() === token &&
      isTokenActive(entry, nowMs)
  );
}

function removeDevice(deviceIdInput) {
  const deviceId = normalizeDeviceId(deviceIdInput);
  const registry = loadRegistry();
  if (!registry.devices[deviceId]) {
    return { removed: false, device_id: deviceId };
  }
  delete registry.devices[deviceId];
  saveRegistry(registry);
  return { removed: true, device_id: deviceId };
}

function listKnownHosts() {
  const registry = loadRegistry();
  return Object.entries(registry.devices || {}).map(([deviceId, record]) => {
    const active = activeTokenRecord(record);
    return {
      device_id: deviceId,
      first_seen: record.first_seen,
      token_active: Boolean(active),
      expires_at: active?.expires_at || null,
      token_count: Array.isArray(record.tokens) ? record.tokens.length : 0,
    };
  });
}

module.exports = {
  registerDevice,
  validateDeviceToken,
  removeDevice,
  listKnownHosts,
  loadRegistry,
  activeTokenRecord,
  isTokenActive,
  REGISTRY_PATH,
  TOKEN_TTL_MS,
};
