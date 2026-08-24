const path = require('path');
const express = require('express');
const cors = require('cors');
const helmet = require('helmet');

const apiRoutes = require('./routes/api');
const plainRoutes = require('./routes/plain');
const walletRoutes = require('./routes/wallet');
const { clientIpHandler } = require('./routes/api');
const {
  clientIpJsonHandler,
  lookupIpJsonHandler,
} = require('./routes/ipJson');

const PORT = Number(process.env.PORT) || 3000;
const HOST = process.env.HOST || '127.0.0.1';

const app = express();

app.set('trust proxy', true);

app.use(
  helmet({
    contentSecurityPolicy: false,
    crossOriginEmbedderPolicy: false,
  })
);

app.use(
  cors({
    origin: [
      'https://www.alturahost.net',
      'http://www.alturahost.net',
      'https://alturahost.net',
      'http://alturahost.net',
      'https://api.alturahost.net',
      'http://api.alturahost.net',
      'https://ip.alturahost.net',
      'http://ip.alturahost.net',
    ],
    methods: ['GET', 'HEAD', 'OPTIONS', 'POST'],
  })
);

app.use(express.json({ limit: '1mb' }));

app.get('/ip', clientIpHandler);
app.get('/json', clientIpJsonHandler);
app.get('/:address/json', lookupIpJsonHandler);
app.use('/api/v1/wallet', walletRoutes);
app.use('/api/v1', apiRoutes);
app.use('/plain', plainRoutes);

const publicDir = path.join(__dirname, '..', 'public');
for (const page of ['privacy', 'cookies', 'terms']) {
  app.get(`/${page}`, (_req, res) => {
    res.sendFile(path.join(publicDir, `${page}.html`));
  });
}

app.use(
  express.static(publicDir, {
    maxAge: process.env.NODE_ENV === 'production' ? '1h' : 0,
    etag: true,
    setHeaders(res, filePath) {
      if (filePath.endsWith('.html') || filePath.endsWith('.js')) {
        res.setHeader('Cache-Control', 'no-cache');
      }
    },
  })
);

app.get('/health', (_req, res) => {
  res.json({ status: 'ok' });
});

app.use((req, res) => {
  if (
    req.path.startsWith('/api/') ||
    req.path === '/plain' ||
    req.path.endsWith('/json') ||
    req.path === '/privacy' ||
    req.path === '/cookies' ||
    req.path === '/terms'
  ) {
    return res.status(404).json({ error: 'Not found' });
  }

  res.status(404).sendFile(path.join(__dirname, '..', 'public', 'index.html'));
});

app.listen(PORT, HOST, () => {
  console.log(`AlturaHost IP checker listening on http://${HOST}:${PORT}`);
});
