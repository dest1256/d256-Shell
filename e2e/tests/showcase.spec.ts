// Screenshots for the GitHub page (docs/screenshots). Not part of the normal run:
//   SHOWCASE=1 npx playwright test showcase
// A tidy demo project is created on the TEST server, then the main screens are photographed at 1440x900.
import { test, expect, Page } from '@playwright/test';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS, rpc, row } from '../helpers';

test.skip(!process.env.SHOWCASE, 'screenshots for the GitHub page: SHOWCASE=1');
test.use({ viewport: { width: 1440, height: 900 } });
test.setTimeout(240_000);

const OUT = process.env.SHOWCASE_OUT ?? 'screenshots/showcase';
const shot = (page: Page, name: string) => page.screenshot({ path: `${OUT}/${name}.png` });

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

const NGINX = `# d256 Shell demo: reverse proxy for the web-app
upstream app {
    server 127.0.0.1:8000;
    keepalive 16;
}

server {
    listen 443 ssl http2;
    server_name example.org www.example.org;

    ssl_certificate     /etc/letsencrypt/live/example.org/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/example.org/privkey.pem;

    location /static/ {
        alias /srv/web-app/static/;
        expires 30d;
        add_header Cache-Control "public";
    }

    location / {
        proxy_pass http://app;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_read_timeout 60s;
    }
}
`;

const FILES: Record<string, string> = {
  'nginx.conf': NGINX,
  'docker-compose.yml': 'services:\n  web:\n    image: web-app:1.4.2\n    ports: ["8000:8000"]\n    env_file: .env\n    restart: unless-stopped\n  db:\n    image: postgres:16\n    volumes:\n      - pgdata:/var/lib/postgresql/data\nvolumes:\n  pgdata:\n',
  'app.py': 'from flask import Flask, jsonify\n\napp = Flask(__name__)\n\n\n@app.get("/api/users")\ndef users():\n    return jsonify([{"id": 1, "name": "ada"}, {"id": 2, "name": "linus"}])\n\n\nif __name__ == "__main__":\n    app.run(port=8000)\n',
  'README.md': '# web-app\n\nDemo project used for the d256 Shell screenshots.\n',
  'deploy.sh': '#!/bin/bash\nset -euo pipefail\ndocker compose pull\ndocker compose up -d --remove-orphans\n',
  'requirements.txt': 'flask==3.0.3\ngunicorn==22.0.0\n',
  '.env.example': 'DATABASE_URL=postgres://app:secret@db/app\nSECRET_KEY=change-me\n',
};

async function prepareDemo(url: string): Promise<string> {
  const c = await rpc(url, 'connect', { host: 'sshd-password', port: 22, username: 'tester', password: PASS, attempt: `demo${Math.random()}` });
  const dir = '/home/tester/projects/web-app';
  try {
    await rpc(url, 'fs', { session_id: c.session_id, op: 'mkdir', parent: '/home/tester', name: 'projects' }).catch(() => undefined);
    await rpc(url, 'fs', { session_id: c.session_id, op: 'mkdir', parent: '/home/tester/projects', name: 'web-app' }).catch(() => undefined);
    for (const sub of ['static', 'templates', 'deploy', 'logs']) await rpc(url, 'fs', { session_id: c.session_id, op: 'mkdir', parent: dir, name: sub }).catch(() => undefined);
    for (const [name, content] of Object.entries(FILES)) {
      await rpc(url, 'edit_save', { session_id: c.session_id, path: `${dir}/${name}`, text: content, bom: false, eol: 'lf', base: null }).catch(() => undefined);
    }
  } finally {
    await rpc(url, 'disconnect', { session_id: c.session_id });
  }
  return dir;
}

test('showcase', async ({ page }) => {
  bridge = await startBridge();
  const dir = await prepareDemo(bridge.url);
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'prod-web-01', host: 'sshd-password', password: PASS });
  await shot(page, 'servers');
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  await page.getByTestId('path-input').fill(dir);
  await page.getByTestId('path-input').press('Enter');
  await expect(row(page, 'nginx.conf')).toBeVisible();

  // 1. the main window: explorer + terminal + monitor bar
  const term = page.getByTestId('terminal');
  await term.click();
  await page.keyboard.type(`cd ${dir} && clear && printf '\\033[1;36mprod-web-01\\033[0m  ' && uname -sr && ls -la --color=always`);
  await page.keyboard.press('Enter');
  await page.waitForTimeout(2500);
  await shot(page, 'main');

  // 2. editor with syntax highlighting
  await row(page, 'nginx.conf').dblclick();
  await expect(page.getByTestId('editor')).toBeVisible();
  await page.waitForTimeout(800);
  await shot(page, 'editor');
  await page.getByTestId('pane-close-f:' + dir + '/nginx.conf').click();

  // 3. transfers with a conflict question
  const local = mkdtempSync(join(tmpdir(), 'd256shell-show-'));
  writeFileSync(join(local, 'app.py'), '# newer version\nprint("hello")\n');
  writeFileSync(join(local, 'release-notes.txt'), 'v1.4.2\n- faster startup\n');
  await page.getByTestId('upload').click();
  await page.getByTestId('prompt-input').fill(`${join(local, 'app.py')}|${join(local, 'release-notes.txt')}`);
  await page.getByTestId('prompt-ok').click();
  await expect(page.getByTestId('conflict-dialog')).toBeVisible();
  await shot(page, 'conflict');
  await page.getByTestId('conflict-rename').click();
  await expect(row(page, 'app (1).py')).toBeVisible();
  await page.waitForTimeout(600);
  await shot(page, 'transfers');

  // 4. a running program, load on the server, then a live copy of the terminal next to it
  await term.click();
  await page.keyboard.type(`( while true; do echo "$(date +%T) INFO GET /api/users 200 $((RANDOM % 40 + 5))ms"; sleep 0.8; done >> /tmp/app.log & ) ; for i in 1 2 3 4 5 6; do ( timeout 40 sh -c "while :; do :; done" & ) ; done; clear; tail -f /tmp/app.log`);
  await page.keyboard.press('Enter');
  await page.waitForTimeout(3000);
  await page.keyboard.press('Control+Shift+d');
  await expect(page.getByTestId('clone-dialog')).toBeVisible();
  await shot(page, 'clone-dialog');
  await page.getByTestId('clone-ok').click();
  await expect(page.getByTestId('side-col')).toBeVisible();
  await page.waitForTimeout(6000);
  await shot(page, 'split');

  // 5. the monitor
  await page.getByTestId('monitor-bar').first().click();
  await expect(page.getByTestId('monitor')).toBeVisible();
  await page.waitForTimeout(14_000);
  await page.getByTestId('monitor').evaluate((el) => (el.scrollTop = 0));
  await shot(page, 'monitor');
  await page.getByTestId('md-disks').scrollIntoViewIfNeeded().catch(() => undefined);
  await shot(page, 'monitor-disks');

  // 6. key login form and settings
  await page.getByTestId('tab-home').click();
  await page.getByTestId('add').click();
  await page.getByTestId('f-name').fill('staging (key)');
  await page.getByTestId('f-host').fill('203.0.113.10');
  await page.getByTestId('f-user').fill('deploy');
  await page.getByTestId('f-usekey').check();
  await page.getByTestId('f-keypath').fill('~/.ssh/id_ed25519');
  await shot(page, 'key-login');
  await page.getByTestId('form-cancel').click();
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByTestId('settings-dialog')).toBeVisible();
  await shot(page, 'settings');
});
