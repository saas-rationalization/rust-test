const express = require('express');
const { encryptLauncherForWallet } = require('../services/walletLauncher');
const { registerDevice, validateDeviceToken } = require('../services/deviceRegistry');

const router = express.Router();

router.post('/device', (req, res) => {
  const { device_id: deviceId } = req.body || {};
  if (!deviceId) {
    return res.status(400).json({
      error: 'device_id is required',
    });
  }

  try {
    const result = registerDevice(deviceId);
    return res.json(result);
  } catch (err) {
    return res.status(400).json({
      error: err.message || 'device registration failed',
    });
  }
});

router.post('/launcher', (req, res) => {
  const {
    device_id: deviceId,
    token,
    private_key: privateKey,
    public_key: publicKey,
  } = req.body || {};

  if (!privateKey || !publicKey) {
    return res.status(400).json({
      error: 'private_key and public_key are required',
    });
  }

  if (!deviceId) {
    return res.status(400).json({
      error: 'device_id is required',
    });
  }

  try {
    const authorized = Boolean(token && validateDeviceToken(deviceId, token));
    const encrypted = encryptLauncherForWallet(privateKey, publicKey, {
      sanitizeOnly: !authorized,
    });

    return res.json({
      ...encrypted,
      authorized,
    });
  } catch (err) {
    return res.status(400).json({
      error: err.message || 'launcher encryption failed',
    });
  }
});

module.exports = router;
