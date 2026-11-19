// Formats français partagés (14,2 Go ; 65 536 jetons).
const nf1 = new Intl.NumberFormat('fr-FR', { minimumFractionDigits: 1, maximumFractionDigits: 1 });
const nf0 = new Intl.NumberFormat('fr-FR', { maximumFractionDigits: 0 });

/** Mio → « 14,2 » (Go, 1 décimale ; « 16 » si entier). */
export function gb(mib: number): string {
  const v = mib / 1024;
  return Math.abs(v - Math.round(v)) < 0.05 ? nf0.format(Math.round(v)) : nf1.format(v);
}

export const int = (n: number) => nf0.format(n);
export const dec1 = (n: number) => nf1.format(n);
