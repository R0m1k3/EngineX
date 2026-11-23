const $ = (s) => document.querySelector(s);
const $$ = (s) => [...document.querySelectorAll(s)];
const ok = (c, m) => { if (!c) throw new Error(m); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const need = (s) => { const e = typeof s === 'string' ? $(s) : s; if (!e) throw new Error('élément ' + s + ' introuvable'); return e; };
const visible = (s) => {
  const e = typeof s === 'string' ? $(s) : s;
  if (!e || !e.isConnected) return false;
  const r = e.getBoundingClientRect();
  if (r.width < 1 || r.height < 1) return false;
  for (let n = e; n && n.nodeType === 1; n = n.parentElement) {
    const st = getComputedStyle(n);
    if (st.display === 'none' || st.visibility === 'hidden' || +st.opacity === 0) return false;
    if (n.hidden) return false;
  }
  return true;
};
const box = (s) => need(s).getBoundingClientRect();
const txt = (s) => need(s).textContent.replace(/\s+/g, ' ').trim();
const click = async (s) => { need(s).click(); await sleep(80); };
const type = async (s, v) => {
  const e = need(s);
  e.focus();
  e.value = v;
  e.dispatchEvent(new Event('input', { bubbles: true }));
  e.dispatchEvent(new Event('change', { bubbles: true }));
  e.dispatchEvent(new Event('blur'));
  e.dispatchEvent(new FocusEvent('focusout', { bubbles: true }));
  await sleep(60);
};
const key = async (k, target) => {
  const t = target ? need(target) : (document.activeElement || document.body);
  for (const ty of ['keydown', 'keyup']) {
    t.dispatchEvent(new KeyboardEvent(ty, { key: k, code: k.length === 1 && /\d/.test(k) ? 'Digit' + k : k, bubbles: true, cancelable: true }));
  }
  await sleep(80);
};
// « 1 832,50 € », « 1,832.50 », « 355.75 » -> nombre
const num = (s) => {
  let t = String(s).replace(/[\s\u00a0\u202f€$]/g, '');
  const m = t.match(/-?[\d.,]+/);
  if (!m) return NaN;
  t = m[0];
  if (t.includes(',') && t.includes('.')) t = t.indexOf(',') < t.indexOf('.') ? t.replace(/,/g, '') : t.replace(/\./g, '').replace(',', '.');
  else if (t.includes(',')) t = t.replace(',', '.');
  return parseFloat(t);
};
const near = (a, b, eps) => Math.abs(a - b) <= (eps ?? 0.005);
const noHScroll = () => {
  const w = document.documentElement.scrollWidth, v = window.innerWidth;
  ok(w <= v + 1, 'la page déborde en largeur : ' + w + ' px pour un écran de ' + v + ' px');
};
const rgb = (c) => {
  const cv = document.createElement('canvas'); cv.width = cv.height = 1;
  const x = cv.getContext('2d'); x.clearRect(0, 0, 1, 1); x.fillStyle = '#000'; x.fillStyle = c; x.fillRect(0, 0, 1, 1);
  const d = x.getImageData(0, 0, 1, 1).data; return [d[0], d[1], d[2], d[3] / 255];
};
const lum = ([r, g, b]) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
const bgOf = (e) => {
  for (let n = e; n; n = n.parentElement) {
    const st = getComputedStyle(n);
    if (st.backgroundImage && st.backgroundImage !== 'none') return null;
    const c = rgb(st.backgroundColor);
    if (st.backgroundColor !== 'transparent' && c[3] >= 0.5) return c;
  }
  return [255, 255, 255, 1];
};
const contrast = (e) => {
  const fg = rgb(getComputedStyle(e).color), bg = bgOf(e);
  if (!bg) return null;
  const a = lum(fg), b = lum(bg);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
};
const until = async (cond, ms, msg) => {
  const end = Date.now() + ms;
  while (Date.now() < end) { if (cond()) return; await sleep(50); }
  throw new Error(msg);
};
