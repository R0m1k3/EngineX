// Tailles en octets au format français de la direction (14,2 Go ; 461 Mo ; 12 Ko).
import { gb, int } from '../../lib/ui/format';

export function octets(n: number): string {
  if (n >= 1 << 30) return `${gb(n / (1 << 20))} Go`;
  if (n >= 1 << 20) return `${int(n / (1 << 20))} Mo`;
  return `${int(Math.max(0, n / 1024))} Ko`;
}
