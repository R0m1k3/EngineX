<script lang="ts" generics="T extends string">
  // Choix exclusif court (Réflexion none/low/medium/high, thème, taille du texte).
  let {
    value = $bindable(),
    options,
    label,
    size = 'md',
    onchange,
  }: {
    value: T;
    options: { value: T; label: string }[];
    label: string;
    size?: 'md' | 'lg';
    onchange?: (v: T) => void;
  } = $props();
</script>

<div class="seg {size}" role="group" aria-label={label}>
  {#each options as o (o.value)}
    <button type="button" aria-pressed={value === o.value} onclick={() => { value = o.value; onchange?.(o.value); }}>{o.label}</button>
  {/each}
</div>

<style>
  .seg { display: inline-flex; border: 1px solid var(--line-strong); border-radius: var(--r); overflow: hidden; }
  button {
    height: 30px;
    min-height: 0;
    padding: 0 var(--s3);
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    filter: none;
    cursor: pointer;
  }
  button + button { border-left: 1px solid var(--line-strong); }
  button:hover { color: var(--text); }
  button[aria-pressed='true'] { background: var(--accent-soft); color: var(--accent); font-weight: 600; }
  .lg button { height: 42px; padding: 0 var(--s4); font-size: var(--fs-md); }
</style>
