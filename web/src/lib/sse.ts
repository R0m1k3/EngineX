// Flux SSE d'une session. EventSource se reconnecte seul ; le serveur
// renvoie alors un `snapshot` complet, qui suffit à se resynchroniser.

export type Handlers = Record<string, (data: any) => void>;

export function openEvents(url: string, handlers: Handlers, onState?: (open: boolean) => void): () => void {
  const es = new EventSource(url, { withCredentials: true });
  for (const [name, fn] of Object.entries(handlers)) {
    es.addEventListener(name, (ev) => {
      try {
        fn(JSON.parse((ev as MessageEvent).data));
      } catch (err) {
        console.error(`événement ${name} illisible`, err);
      }
    });
  }
  es.onopen = () => onState?.(true);
  es.onerror = () => onState?.(false);
  return () => es.close();
}
