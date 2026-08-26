import fs from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';
import { createRequire } from 'node:module';

function parseArgs(argv) {
  const args = {};
  for (let i = 2; i < argv.length; i += 1) {
    const key = argv[i];
    if (!key.startsWith('--')) continue;
    const value = argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[++i] : true;
    args[key.slice(2)] = value;
  }
  return args;
}

const args = parseArgs(process.argv);
if (!args.manifest || !args['modules-root'] || !args.edge) {
  throw new Error('Usage: node mirror-external-links.mjs --manifest <path> --modules-root <runner> --edge <msedge.exe> [--domain host] [--limit n] [--probe]');
}

const requireFromRunner = createRequire(path.join(path.resolve(args['modules-root']), 'package.json'));
const { chromium } = requireFromRunner('playwright-core');
const TurndownService = requireFromRunner('turndown');
const manifestPath = path.resolve(args.manifest);
const vaultRoot = path.dirname(manifestPath);
const knowledgeBaseUrl = 'https://www.yuque.com/sgyxy/cangyun';
const capturedAt = new Date().toISOString();

function sha256(value) {
  return crypto.createHash('sha256').update(value, 'utf8').digest('hex');
}

function yaml(value) {
  return JSON.stringify(value ?? '');
}

function isoFromUnix(value) {
  return value ? new Date(Number(value) * 1000).toISOString() : '';
}

function cleanMarkdown(value) {
  return value
    .replace(/\r/g, '')
    .replace(/\n{4,}/g, '\n\n\n')
    .replace(/[ \t]+\n/g, '\n')
    .trim();
}

async function fetchJson(url) {
  const response = await fetch(url, {
    headers: {
      accept: 'application/json,text/plain,*/*',
      referer: 'https://www.bilibili.com/',
      'user-agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131 Safari/537.36',
    },
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}: ${url}`);
  return { status: response.status, json: await response.json() };
}

async function captureBilibili(url) {
  const bvid = new URL(url).pathname.match(/\/video\/(BV[0-9A-Za-z]+)/i)?.[1];
  if (!bvid) throw new Error('Cannot parse Bilibili BV id');
  const viewResult = await fetchJson(`https://api.bilibili.com/x/web-interface/view?bvid=${bvid}`);
  if (viewResult.json.code !== 0) throw new Error(`Bilibili API ${viewResult.json.code}: ${viewResult.json.message}`);
  const data = viewResult.json.data;
  const sections = [
    `# ${data.title}`,
    '',
    data.desc || '',
    '',
    '## 视频信息',
    '',
    `- UP 主：${data.owner?.name || '未知'}`,
    `- BV 号：${data.bvid}`,
    `- 时长：${data.duration ?? 0} 秒`,
    `- 分 P 数：${data.videos ?? data.pages?.length ?? 1}`,
    `- 播放：${data.stat?.view ?? 0}`,
    `- 点赞：${data.stat?.like ?? 0}`,
  ];
  let subtitleCount = 0;
  for (const part of data.pages || []) {
    try {
      const player = await fetchJson(`https://api.bilibili.com/x/player/v2?bvid=${bvid}&cid=${part.cid}`);
      const subtitles = player.json?.data?.subtitle?.subtitles || [];
      if (!subtitles.length) continue;
      sections.push('', `## 字幕：${part.part || `P${part.page}`}`, '');
      for (const subtitle of subtitles) {
        const subtitleUrl = subtitle.subtitle_url?.startsWith('//') ? `https:${subtitle.subtitle_url}` : subtitle.subtitle_url;
        if (!subtitleUrl) continue;
        const subtitleData = await fetchJson(subtitleUrl);
        const lines = (subtitleData.json.body || []).map((line) => line.content?.trim()).filter(Boolean);
        if (lines.length) {
          sections.push(...lines.map((line) => `${line}`), '');
          subtitleCount += 1;
          break;
        }
      }
    } catch {
      // Metadata remains useful when subtitles are unavailable.
    }
  }
  const markdown = cleanMarkdown(sections.join('\n'));
  return {
    status: markdown.length >= 300 ? 'full' : 'metadata_only',
    httpStatus: viewResult.status,
    pageTitle: data.title || '',
    author: data.owner?.name || '',
    description: data.desc || '',
    publishedAt: isoFromUnix(data.pubdate),
    updatedAt: '',
    canonicalUrl: `https://www.bilibili.com/video/${data.bvid}`,
    markdown,
    subtitleCount,
    extraction: subtitleCount ? 'bilibili_public_api_with_subtitles' : 'bilibili_public_api_metadata',
  };
}

function createTurndown() {
  const service = new TurndownService({ headingStyle: 'atx', bulletListMarker: '-', codeBlockStyle: 'fenced' });
  service.remove(['script', 'style', 'noscript', 'svg', 'canvas', 'form', 'button']);
  service.addRule('dropEmptyLinks', {
    filter: (node) => node.nodeName === 'A' && !(node.textContent || '').trim() && !node.querySelector('img'),
    replacement: () => '',
  });
  return service;
}

async function waitForRenderedContent(page) {
  let previous = -1;
  let stable = 0;
  for (let i = 0; i < 10; i += 1) {
    const length = await page.evaluate(() => document.body?.innerText?.trim().length || 0);
    if (length === previous && length > 200) stable += 1;
    else stable = 0;
    if (stable >= 2) break;
    previous = length;
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await page.waitForTimeout(700);
  }
  await page.evaluate(() => window.scrollTo(0, 0));
}

async function captureRendered(browser, url) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, locale: 'zh-CN' });
  let response;
  try {
    response = await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 45_000 });
    await waitForRenderedContent(page);
    const extracted = await page.evaluate(() => {
      const getMeta = (...names) => {
        for (const name of names) {
          const node = document.querySelector(`meta[name="${name}"], meta[property="${name}"], meta[itemprop="${name}"]`);
          const value = node?.getAttribute('content')?.trim();
          if (value) return value;
        }
        return '';
      };
      const jsonLd = [...document.querySelectorAll('script[type="application/ld+json"]')].map((node) => {
        try { return JSON.parse(node.textContent); } catch { return null; }
      }).flatMap((item) => Array.isArray(item) ? item : item?.['@graph'] || [item]).filter(Boolean);
      const articleData = jsonLd.find((item) => /Article|VideoObject|CreativeWork/i.test(String(item?.['@type'] || ''))) || {};
      const candidates = [...new Set([
        ...document.querySelectorAll('article, main, [role="main"], .markdown-body, .article-content, .post-content, .content-body, .rich-text, .page-content, #app'),
      ])];
      const score = (node) => {
        const text = node.innerText?.trim() || '';
        const links = node.querySelectorAll('a').length;
        const paragraphs = node.querySelectorAll('p,li,pre,table,h1,h2,h3').length;
        return text.length + paragraphs * 40 - Math.max(0, links - paragraphs * 2) * 10;
      };
      let selected = candidates.sort((a, b) => score(b) - score(a))[0] || document.body;
      const clone = selected.cloneNode(true);
      clone.querySelectorAll('script,style,noscript,svg,canvas,form,button,nav,aside,header,footer,[role="navigation"],[aria-hidden="true"],[class*="comment"],[id*="comment"],.c-sidebar').forEach((node) => node.remove());
      clone.querySelectorAll('[href]').forEach((node) => { if (node.href) node.setAttribute('href', node.href); });
      clone.querySelectorAll('[src]').forEach((node) => {
        const resolved = node.currentSrc || node.src;
        if (resolved) node.setAttribute('src', resolved);
      });
      return {
        html: clone.innerHTML,
        text: clone.innerText?.trim() || '',
        pageTitle: getMeta('og:title', 'twitter:title') || articleData.headline || articleData.name || document.title || document.querySelector('h1')?.innerText?.trim() || '',
        author: getMeta('author', 'article:author') || articleData.author?.name || document.querySelector('[rel="author"], .author-name, a[href*="/author/"], [class*="author"] [class*="name"], [class*="author"] a')?.textContent?.trim() || '',
        description: getMeta('description', 'og:description', 'twitter:description'),
        publishedAt: getMeta('article:published_time', 'datePublished', 'publishdate') || articleData.datePublished || '',
        updatedAt: getMeta('article:modified_time', 'dateModified', 'last-modified') || articleData.dateModified || '',
        canonicalUrl: document.querySelector('link[rel="canonical"]')?.href || location.href,
        candidateInfo: candidates.slice(0, 20).map((node) => ({ tag: node.tagName, id: node.id, className: String(node.className).slice(0, 160), length: node.innerText?.trim().length || 0 })),
      };
    });
    const markdown = cleanMarkdown(createTurndown().turndown(extracted.html));
    const meaningfulLength = extracted.text.replace(/\s+/g, '').length;
    const host = new URL(url).hostname.toLowerCase();
    const shellOnly = (host === 'docs.qq.com' && /Ctrl\+Alt\+SHIFT|欢迎使用腾讯文档/.test(extracted.text))
      || (host.endsWith('kdocs.cn') && meaningfulLength < 250)
      || (host === 'dps.btcsg.top' && /create-react-app/i.test(extracted.description));
    const isFull = !shellOnly && meaningfulLength >= 250 && markdown.length >= 250;
    const hasMetadata = Boolean(extracted.pageTitle || extracted.description);
    let status = isFull ? 'full' : hasMetadata ? 'metadata_only' : 'failed';
    let error = isFull || hasMetadata ? '' : `Rendered text too short (${meaningfulLength} characters)`;
    if (/Security Verification|安全验证/i.test(extracted.pageTitle)) {
      status = 'failed';
      error = 'Blocked by security verification';
    }
    if ((response?.status() || 0) >= 400) {
      status = 'failed';
      error = `HTTP ${response.status()}`;
    }
    let author = extracted.author;
    if (!author && host === 'www.jx3box.com') {
      author = extracted.text.match(/联合创作(.{1,20}?)UP/)?.[1]?.trim() || '';
    }
    return {
      status,
      httpStatus: response?.status() || 0,
      ...extracted,
      author,
      markdown: isFull ? markdown : cleanMarkdown([`# ${extracted.pageTitle}`, '', extracted.description].join('\n')),
      subtitleCount: 0,
      extraction: 'edge_rendered_dom',
      error,
    };
  } finally {
    await page.close();
  }
}

function frontmatter(entry, capture) {
  const sourceUrl = entry.source;
  const rows = [
    '---',
    `title: ${yaml(entry.title)}`,
    'kind: external_mirror',
    `season: ${yaml(entry.season)}`,
    `category: ${yaml(entry.category)}`,
    `source_url: ${yaml(sourceUrl)}`,
    `source_site: ${yaml(new URL(sourceUrl).hostname.toLowerCase())}`,
    `canonical_url: ${yaml(capture.canonicalUrl || sourceUrl)}`,
    `yuque_book_url: ${yaml(entry.yuque_book_url || knowledgeBaseUrl)}`,
    `yuque_book_id: ${yaml(entry.yuque_book_id || '')}`,
    `yuque_book_title: ${yaml(entry.yuque_book_title || '')}`,
    `yuque_book_owner_name: ${yaml(entry.yuque_book_owner_name || '')}`,
    `yuque_book_owner_login: ${yaml(entry.yuque_book_owner_login || '')}`,
    `yuque_book_owner_id: ${yaml(entry.yuque_book_owner_id || '')}`,
    `yuque_catalog_path: ${yaml(entry.yuque_catalog_path || `${entry.season} / ${entry.category} / ${entry.title}`)}`,
    'yuque_entry_type: external_link',
    `yuque_url: ${yaml(entry.yuque_url || knowledgeBaseUrl)}`,
    `external_url: ${yaml(sourceUrl)}`,
    `source_title: ${yaml(capture.pageTitle || '')}`,
    `source_author: ${yaml(capture.author || '')}`,
    `source_description: ${yaml(capture.description || '')}`,
    `source_published_at: ${yaml(capture.publishedAt || '')}`,
    `source_updated_at: ${yaml(capture.updatedAt || '')}`,
    `captured_at: ${yaml(capturedAt)}`,
    `mirror_status: ${capture.status}`,
    `retrieval_eligible: ${capture.status !== 'failed'}`,
    `retrieval_scope: ${capture.status === 'full' ? 'full_text' : capture.status === 'metadata_only' ? 'metadata_only' : 'none'}`,
    `extraction_method: ${capture.extraction}`,
    `http_status: ${capture.httpStatus || 0}`,
    `content_sha256: ${yaml(sha256(capture.markdown || ''))}`,
  ];
  if (capture.subtitleCount) rows.push(`subtitle_track_count: ${capture.subtitleCount}`);
  if (capture.error) rows.push(`mirror_error: ${yaml(capture.error)}`);
  rows.push('---');
  return rows.join('\n');
}

function documentBody(entry, capture) {
  const sourceUrl = entry.source;
  const provenance = [
    '## 来源与溯源',
    '',
    `- [打开外部原文](${sourceUrl})`,
    `- [打开语雀知识库](${entry.yuque_url || knowledgeBaseUrl})`,
    `- 语雀目录位置：${entry.yuque_catalog_path || `${entry.season} / ${entry.category} / ${entry.title}`}`,
    `- 本地抓取时间：${capturedAt}`,
  ];
  if (capture.error) provenance.push(`- 抓取说明：${capture.error}`);
  const mirrored = capture.markdown || `# ${entry.title}\n\n正文未能公开抓取，请通过上方原文链接访问。`;
  return `${mirrored}\n\n---\n\n${provenance.join('\n')}\n`;
}

async function main() {
  const manifest = JSON.parse(await fs.readFile(manifestPath, 'utf8'));
  let entries = manifest.entries.filter((entry) => entry.kind === 'external_link' || entry.kind === 'external_mirror');
  if (args.domain) entries = entries.filter((entry) => new URL(entry.source).hostname === args.domain);
  if (args.limit) entries = entries.slice(0, Number(args.limit));
  const uniqueUrls = [...new Set(entries.map((entry) => entry.source))];
  const browserUrls = uniqueUrls.filter((url) => !new URL(url).hostname.endsWith('bilibili.com'));
  const browser = browserUrls.length
    ? await chromium.launch({ executablePath: path.resolve(args.edge), headless: true, args: ['--no-first-run', '--disable-features=msEdgeFirstRunExperience'] })
    : null;
  const captures = new Map();
  try {
    let nextIndex = 0;
    const worker = async () => {
      while (true) {
        const index = nextIndex++;
        if (index >= uniqueUrls.length) return;
        const url = uniqueUrls[index];
        let capture;
        try {
          capture = new URL(url).hostname.endsWith('bilibili.com')
            ? await captureBilibili(url)
            : await captureRendered(browser, url);
        } catch (error) {
          capture = {
            status: 'failed', httpStatus: 0, pageTitle: '', author: '', description: '',
            publishedAt: '', updatedAt: '', canonicalUrl: url, markdown: '', subtitleCount: 0,
            extraction: 'failed', error: String(error?.message || error),
          };
        }
        captures.set(url, capture);
        process.stdout.write(`[${index + 1}/${uniqueUrls.length}] ${capture.status.padEnd(13)} ${new URL(url).hostname} ${url}\n`);
        if (args.probe) process.stdout.write(`${JSON.stringify({ ...capture, html: undefined, text: undefined, markdown: capture.markdown.slice(0, 800) }, null, 2)}\n`);
      }
    };
    const concurrency = Math.max(1, Math.min(Number(args.concurrency || 3), uniqueUrls.length));
    await Promise.all(Array.from({ length: concurrency }, () => worker()));
  } finally {
    if (browser) await browser.close();
  }

  if (!args.probe) {
    for (const entry of entries) {
      const capture = captures.get(entry.source);
      const outputPath = path.join(vaultRoot, entry.output);
      const content = `${frontmatter(entry, capture)}\n\n${documentBody(entry, capture)}`;
      await fs.writeFile(outputPath, content, 'utf8');
      entry.kind = 'external_mirror';
      entry.mirror_status = capture.status;
      entry.captured_at = capturedAt;
      entry.content_sha256 = sha256(capture.markdown || '');
      entry.source_title = capture.pageTitle || '';
      entry.source_author = capture.author || '';
      entry.source_published_at = capture.publishedAt || '';
      entry.source_updated_at = capture.updatedAt || '';
      entry.canonical_url = capture.canonicalUrl || entry.source;
      entry.extraction_method = capture.extraction;
      entry.http_status = capture.httpStatus || 0;
      entry.mirror_error = capture.error || '';
    }
    manifest.schema_version = Math.max(Number(manifest.schema_version || 1), 3);
    manifest.external_mirror_generated_at = capturedAt;
    await fs.writeFile(manifestPath, JSON.stringify(manifest, null, 2), 'utf8');
    let previousCaptures = [];
    try {
      const previous = JSON.parse(await fs.readFile(path.join(vaultRoot, '_external-mirror-manifest.json'), 'utf8'));
      previousCaptures = previous.captures || [];
    } catch {
      previousCaptures = [];
    }
    const allCaptureMap = new Map(previousCaptures.map((capture) => [capture.url, capture]));
    for (const [url, capture] of captures) allCaptureMap.set(url, capture);
    const allExternalEntries = manifest.entries.filter((entry) => entry.kind === 'external_mirror');
    const allExternalUrls = [...new Set(allExternalEntries.map((entry) => entry.source))];
    const mirrorManifest = {
      schema_version: 1,
      generated_at: capturedAt,
      total_entries: allExternalEntries.length,
      unique_urls: allExternalUrls.length,
      counts: Object.fromEntries(['full', 'metadata_only', 'failed'].map((status) => [status, allExternalEntries.filter((entry) => entry.mirror_status === status).length])),
      captures: allExternalUrls.map((url) => {
        const { html, text, markdown, candidateInfo, ...capture } = allCaptureMap.get(url) || {};
        return { url, ...capture };
      }),
    };
    await fs.writeFile(path.join(vaultRoot, '_external-mirror-manifest.json'), JSON.stringify(mirrorManifest, null, 2), 'utf8');
  }
}

await main();
