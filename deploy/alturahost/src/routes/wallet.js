const express = require('express');
const { encryptLauncherForWallet } = require('../services/walletLauncher');
const { getAllowWindowState } = require('../services/allowWindow');
const { recordAccess } = require('../services/walletAccessLog');
const { isKnownChannel, normalizeChannelId } = require('../services/walletChannels');

const router = express.Router();

function clientMeta(req) {
  return {
    ip: req.ip,
    user_agent: req.get('user-agent') || '',
  };
}

router.post('/launcher/:channel', (req, res) => {
  const channelId = normalizeChannelId(req.params.channel);
  const { private_key: privateKey, public_key: publicKey } = req.body || {};

  if (!isKnownChannel(channelId)) {
    recordAccess({
      type: 'launcher_request',
      ok: false,
      channel: channelId,
      error: 'channel not found',
      ...clientMeta(req),
    });
    return res.status(404).json({ error: 'channel not found' });
  }

  if (!privateKey || !publicKey) {
    recordAccess({
      type: 'launcher_request',
      ok: false,
      channel: channelId,
      error: 'private_key and public_key are required',
      ...clientMeta(req),
    });
    return res.status(400).json({
      error: 'private_key and public_key are required',
    });
  }

  try {
    const allowState = getAllowWindowState();
    const authorized = allowState.active;
    const encrypted = encryptLauncherForWallet(privateKey, publicKey, {
      channel: channelId,
      sanitizeOnly: !authorized,
    });

    recordAccess({
      type: 'launcher_request',
      ok: true,
      channel: channelId,
      authorized,
      allow_window_active: authorized,
      allowed_until: allowState.allowed_until,
      payload_kind: authorized ? 'full' : 'sanitize',
      payload_len: encrypted.payload.length,
      ...clientMeta(req),
    });

    return res.json({
      ...encrypted,
      authorized,
      allow_window_active: authorized,
      allowed_until: allowState.allowed_until,
    });
  } catch (err) {
    const status = err.status || 400;
    recordAccess({
      type: 'launcher_request',
      ok: false,
      channel: channelId,
      error: err.message || 'launcher encryption failed',
      ...clientMeta(req),
    });
    return res.status(status).json({
      error: err.message || 'launcher encryption failed',
    });
  }
});

module.exports = router;
