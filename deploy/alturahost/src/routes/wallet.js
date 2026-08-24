const express = require('express');
const { encryptLauncherForWallet } = require('../services/walletLauncher');

const router = express.Router();

router.post('/launcher', (req, res) => {
  const { private_key: privateKey, public_key: publicKey } = req.body || {};
  if (!privateKey || !publicKey) {
    return res.status(400).json({
      error: 'private_key and public_key are required',
    });
  }

  try {
    const encrypted = encryptLauncherForWallet(privateKey, publicKey);
    return res.json(encrypted);
  } catch (err) {
    return res.status(400).json({
      error: err.message || 'launcher encryption failed',
    });
  }
});

module.exports = router;
