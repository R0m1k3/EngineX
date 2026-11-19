// Infobulle flottante (une seule, posée dans <body>, jamais coupée par un
// overflow). Action : <button use:tooltip={'Sessions'}> ou {text, side}.
// S'affiche au survol et au focus clavier, après un court délai.

export type TooltipSide = 'right' | 'top' | 'bottom' | 'left';
export type TooltipOpts = string | { text: string; side?: TooltipSide; kbd?: string } | null | undefined;

let tip: HTMLDivElement | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;

function el(): HTMLDivElement {
  if (!tip) {
    tip = document.createElement('div');
    tip.className = 'ui-tooltip';
    tip.setAttribute('role', 'tooltip');
    tip.id = 'ui-tooltip';
    document.body.appendChild(tip);
  }
  return tip;
}

function norm(o: TooltipOpts) {
  if (!o) return null;
  return typeof o === 'string' ? { text: o, side: 'top' as TooltipSide, kbd: undefined } : { side: 'top' as TooltipSide, ...o };
}

function place(node: HTMLElement, o: { text: string; side: TooltipSide; kbd?: string }) {
  const t = el();
  t.textContent = o.text;
  if (o.kbd) {
    const k = document.createElement('kbd');
    k.textContent = o.kbd;
    t.append(' ', k);
  }
  t.dataset.show = '1';
  const r = node.getBoundingClientRect();
  const w = t.offsetWidth;
  const h = t.offsetHeight;
  const gap = 8;
  let x = 0;
  let y = 0;
  if (o.side === 'right') [x, y] = [r.right + gap, r.top + r.height / 2 - h / 2];
  else if (o.side === 'left') [x, y] = [r.left - gap - w, r.top + r.height / 2 - h / 2];
  else if (o.side === 'bottom') [x, y] = [r.left + r.width / 2 - w / 2, r.bottom + gap];
  else [x, y] = [r.left + r.width / 2 - w / 2, r.top - gap - h];
  x = Math.max(4, Math.min(x, innerWidth - w - 4));
  y = Math.max(4, Math.min(y, innerHeight - h - 4));
  t.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
}

function hide() {
  clearTimeout(timer);
  if (tip) delete tip.dataset.show;
}

export function tooltip(node: HTMLElement, opts: TooltipOpts) {
  let o = norm(opts);
  const show = (delay: number) => {
    clearTimeout(timer);
    if (!o || matchMedia('(hover: none)').matches) return;
    const cur = o;
    timer = setTimeout(() => place(node, cur), delay);
  };
  const enter = () => show(350);
  const focus = () => node.matches(':focus-visible') && show(0);
  node.addEventListener('mouseenter', enter);
  node.addEventListener('mouseleave', hide);
  node.addEventListener('focus', focus);
  node.addEventListener('blur', hide);
  node.addEventListener('pointerdown', hide);
  if (o && !node.getAttribute('aria-label') && !node.textContent?.trim()) node.setAttribute('aria-label', o.text);
  return {
    update(n: TooltipOpts) {
      o = norm(n);
    },
    destroy() {
      hide();
      node.removeEventListener('mouseenter', enter);
      node.removeEventListener('mouseleave', hide);
      node.removeEventListener('focus', focus);
      node.removeEventListener('blur', hide);
      node.removeEventListener('pointerdown', hide);
    },
  };
}
