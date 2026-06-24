export function renderDiscoveryGuide(baseUrl: string): string {
  const apiBase = baseUrl.replace(/\/+$/, "");
  const publicKeyUrl = `${apiBase}/api/v1/discovery/public-key`;
  const commentsUrl = `${apiBase}/api/v1/comments/current`;
  const metadataJson = JSON.stringify(
    {
      atrium: "v1",
      origin: "https://blog.example.com",
      name: "Blog",
      admin_emails: ["owner@example.com"],
      contact_email: "owner@example.com"
    },
    null,
    2
  );
  const encryptedJson = JSON.stringify(
    {
      atrium: "v1",
      origin: "https://blog.example.com",
      name: "Blog",
      admin_emails: "enc:jwe:<compact-jwe>",
      contact_email: "enc:jwe:<compact-jwe>"
    },
    null,
    2
  );
  const txtRecord =
    '_atrium.blog.example.com TXT "atrium-site={\\"atrium\\":\\"v1\\",\\"origin\\":\\"https://blog.example.com\\",\\"name\\":\\"Blog\\",\\"admin_emails\\":[\\"owner@example.com\\"]}"';

  return `<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Atrium Discovery 接入指南</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f7f8fa;
      --ink: #15191f;
      --muted: #5f6875;
      --line: #d8dde5;
      --panel: #ffffff;
      --panel-2: #eef4f1;
      --accent: #0f766e;
      --accent-2: #8a5a00;
      --code: #111827;
      --code-bg: #f1f5f9;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      background: var(--bg);
      color: var(--ink);
      font: 15px/1.65 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }
    main {
      width: min(1040px, calc(100% - 32px));
      margin: 0 auto;
      padding: 48px 0 72px;
    }
    header {
      border-bottom: 1px solid var(--line);
      padding-bottom: 24px;
      margin-bottom: 28px;
    }
    h1 {
      margin: 0 0 12px;
      font-size: clamp(32px, 5vw, 56px);
      line-height: 1.05;
      letter-spacing: 0;
    }
    h2 {
      margin: 36px 0 12px;
      font-size: 24px;
      line-height: 1.25;
      letter-spacing: 0;
    }
    h3 {
      margin: 22px 0 8px;
      font-size: 17px;
      line-height: 1.35;
      letter-spacing: 0;
    }
    p { margin: 0 0 12px; color: var(--muted); }
    a { color: var(--accent); text-decoration-thickness: 1px; text-underline-offset: 3px; }
    code {
      border: 1px solid var(--line);
      border-radius: 6px;
      background: var(--code-bg);
      color: var(--code);
      padding: 1px 5px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 0.93em;
    }
    pre {
      margin: 12px 0 18px;
      overflow-x: auto;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--code-bg);
      padding: 16px;
      color: var(--code);
    }
    pre code {
      border: 0;
      border-radius: 0;
      background: transparent;
      padding: 0;
      font-size: 13px;
      line-height: 1.55;
      white-space: pre;
    }
    .lead {
      max-width: 760px;
      font-size: 18px;
      color: #374151;
    }
    .grid {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 12px;
      margin: 20px 0 28px;
    }
    .step, .note, .check {
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--panel);
      padding: 16px;
    }
    .step strong {
      display: block;
      margin-bottom: 6px;
      color: var(--ink);
    }
    .note {
      border-color: #c7d7d2;
      background: var(--panel-2);
    }
    .note strong { color: var(--accent); }
    .check {
      display: grid;
      grid-template-columns: 24px 1fr;
      gap: 10px;
      align-items: start;
    }
    .check span {
      display: grid;
      place-items: center;
      width: 24px;
      height: 24px;
      border-radius: 999px;
      background: #d8eee9;
      color: #065f56;
      font-weight: 700;
    }
    .flow {
      display: grid;
      grid-template-columns: 1fr auto 1fr auto 1fr;
      gap: 10px;
      align-items: center;
      margin: 18px 0 26px;
    }
    .flow div {
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--panel);
      padding: 14px;
      min-height: 88px;
    }
    .flow b {
      display: block;
      margin-bottom: 5px;
    }
    .arrow {
      color: var(--accent);
      font-size: 22px;
      font-weight: 700;
    }
    .table {
      width: 100%;
      border-collapse: collapse;
      margin: 12px 0 20px;
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 8px;
      overflow: hidden;
      display: table;
    }
    th, td {
      border-bottom: 1px solid var(--line);
      padding: 10px 12px;
      text-align: left;
      vertical-align: top;
    }
    th { background: #edf2f7; }
    tr:last-child td { border-bottom: 0; }
    .warn {
      border-left: 4px solid var(--accent-2);
      padding: 10px 12px;
      background: #fff7e6;
      color: #4f3900;
      margin: 12px 0 20px;
    }
    footer {
      border-top: 1px solid var(--line);
      margin-top: 40px;
      padding-top: 18px;
      color: var(--muted);
    }
    @media (max-width: 780px) {
      main { width: min(100% - 24px, 1040px); padding-top: 28px; }
      .grid, .flow { grid-template-columns: 1fr; }
      .arrow { display: none; }
      h1 { font-size: 34px; }
    }
  </style>
</head>
<body>
  <main>
    <header>
      <h1>Atrium Discovery 接入指南</h1>
      <p class="lead">当评论组件从未知站点发起 quick mode 请求时，Atrium 可以通过站点自己的元数据文件或 DNS TXT 记录自动建站、绑定 origin，并把管理员邮箱保存为待认领权限。</p>
    </header>

    <section class="flow" aria-label="Discovery flow">
      <div><b>评论组件发起请求</b><p><code>Referer</code> 指向当前页面，API 调用 <code>${escapeHtml(commentsUrl)}</code>。</p></div>
      <span class="arrow">→</span>
      <div><b>Atrium 发现站点</b><p>先读 <code>/.well-known/atrium.json</code>，再查 <code>_atrium.&lt;host&gt;</code> TXT。</p></div>
      <span class="arrow">→</span>
      <div><b>管理员认领</b><p>邮箱匹配 Account 登录用户后，该用户成为 website admin。</p></div>
    </section>

    <section>
      <h2>最快接入</h2>
      <div class="grid">
        <div class="step"><strong>1. 放置元数据</strong><p>在站点根域发布 <code>/.well-known/atrium.json</code>，或添加等价的 DNS TXT 记录。</p></div>
        <div class="step"><strong>2. 可选填写 origin</strong><p><code>origin</code> 可以省略；填写时必须和浏览器实际 <code>Referer</code> 的 origin 完全一致。</p></div>
        <div class="step"><strong>3. 登录认领</strong><p><code>admin_emails</code> 中的邮箱登录 Atrium 后，会自动获得该站点管理权限。</p></div>
      </div>
    </section>

    <section>
      <h2>方式一：well-known 文件</h2>
      <p>在当前站点发布以下文件：</p>
      <pre><code>https://blog.example.com/.well-known/atrium.json</code></pre>
      <pre><code>${escapeHtml(metadataJson)}</code></pre>
    </section>

    <section>
      <h2>方式二：DNS TXT</h2>
      <p>DNS TXT 与文件承载同一份扁平 JSON。TXT 记录只加 <code>atrium-site=</code> 前缀，不需要写 metadata URL、hash 或跳转地址。</p>
      <pre><code>${escapeHtml(txtRecord)}</code></pre>
      <p>如果 TXT 内容超过单段长度，可以拆成同一 TXT record 内的多个字符串；Atrium 会按 DNS 返回的字符串顺序直接拼接，不额外插入空格。</p>
    </section>

    <section>
      <h2>字段说明</h2>
      <table class="table">
        <thead><tr><th>字段</th><th>要求</th></tr></thead>
        <tbody>
          <tr><td><code>atrium</code></td><td>固定为 <code>v1</code>。</td></tr>
          <tr><td><code>origin</code></td><td>可选；填写时必须是 HTTPS origin，并且必须等于请求页面的 origin。省略时 Atrium 使用当前页面 origin。</td></tr>
          <tr><td><code>name</code></td><td>可选；默认使用当前页面 hostname。</td></tr>
          <tr><td><code>admin_emails</code></td><td>必填；JSON array。发现成功后会作为待认领管理员邮箱保存。</td></tr>
          <tr><td><code>contact_email</code></td><td>可选；用于公开联系信息或后续管理流程。</td></tr>
        </tbody>
      </table>
    </section>

    <section>
      <h2>加密敏感字段</h2>
      <p>任意敏感顶层字段都可以改成 <code>enc:jwe:</code> 前缀的 JWE compact string。解密后的明文必须是该字段原本的 JSON 值。</p>
      <pre><code>${escapeHtml(encryptedJson)}</code></pre>
      <div class="note"><strong>加密参数</strong><p>JWE 使用 <code>RSA-OAEP-256</code> 和 <code>A256GCM</code>，并在 protected header 中携带当前 <code>kid</code>。</p></div>
      <h3>获取公钥</h3>
      <pre><code>GET ${escapeHtml(publicKeyUrl)}</code></pre>
      <h3>明文类型必须保持一致</h3>
      <div class="check"><span>✓</span><p><code>admin_emails</code> 加密前的明文是 JSON array，例如 <code>["owner@example.com"]</code>。</p></div>
      <div class="check"><span>✓</span><p><code>contact_email</code> 加密前的明文是 JSON string，例如 <code>"owner@example.com"</code>。</p></div>
    </section>

    <section>
      <h2>请求限制与安全规则</h2>
      <div class="warn">Atrium 只会对 HTTPS origin 做 discovery。well-known 读取禁止跨 origin redirect，远端响应最大 16KB，超时约 2.5 秒。失败会进入短期负缓存。</div>
      <p>已注册 origin 不会触发 discovery；显式 website/page/comments API 不受影响。</p>
    </section>

    <section>
      <h2>排错</h2>
      <table class="table">
        <thead><tr><th>现象</th><th>检查点</th></tr></thead>
        <tbody>
          <tr><td><code>website_not_found</code></td><td>如果填写了 <code>origin</code>，确认它与页面 origin 完全一致；同时确认 metadata 文件或 TXT 可被公网读取。</td></tr>
          <tr><td>加密字段被拒绝</td><td>确认 JWE header 的 <code>alg</code>、<code>enc</code>、<code>kid</code> 正确，且解密后 JSON 类型与字段要求一致。</td></tr>
          <tr><td>管理员看不到站点</td><td>确认 Account 登录邮箱与 <code>admin_emails</code> 中的邮箱一致；邮箱会统一按小写匹配。</td></tr>
          <tr><td>key 冲突</td><td>如果从页面 hostname 推导出的 key 已存在，需要站点管理员手动添加 origin，Atrium 不会自动合并。</td></tr>
        </tbody>
      </table>
    </section>

    <footer>
      Atrium discovery 会保护公开元数据中的人员隐私，但解密后的管理员邮箱会以原文保存到 Atrium 数据库，用于权限认领。
    </footer>
  </main>
</body>
</html>`;
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
