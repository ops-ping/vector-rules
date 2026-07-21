<script>
  import AddressExample from '../examples/AddressExample.svelte';
  import FraudTriage from '../examples/FraudTriage.svelte';
  import Prove from '../examples/Prove.svelte';
  import Semantic from '../examples/Semantic.svelte';
  import Streaming from '../examples/Streaming.svelte';

  let { example = 'address', onnavigate } = $props();

  const examples = [
    { id: 'address', label: 'Address verification', hint: 'canonicalization, reference matching, and organizational policy' },
    { id: 'semantic', label: 'Semantic rules', hint: 'vector predicates and forward chaining' },
    { id: 'fraud', label: 'Fraud triage', hint: 'fitted geometry artifacts + calibrated features + symbolic decisions' },
    { id: 'streaming', label: 'Streaming', hint: 'incremental rules in browser WebAssembly' },
    { id: 'prove', label: 'Proof', hint: 'goal-directed backward chaining' }
  ];
  const exampleIds = new Set(examples.map((item) => item.id));
  let selected = $derived(exampleIds.has(example) ? example : 'address');
</script>

<section class="examples">
  <div class="intro">
    <div>
      <h2>Examples</h2>
      <p class="muted">Isolated demonstrations built on the same reusable runtime and browser APIs.</p>
    </div>
    <nav aria-label="Examples">
      {#each examples as item}
        <button
          class:active={selected === item.id}
          title={item.hint}
          onclick={() => onnavigate(item.id)}
        >{item.label}</button>
      {/each}
    </nav>
  </div>

  {#if selected === 'address'}
    <AddressExample />
  {:else if selected === 'semantic'}
    <Semantic />
  {:else if selected === 'fraud'}
    <FraudTriage />
  {:else if selected === 'streaming'}
    <Streaming />
  {:else if selected === 'prove'}
    <Prove />
  {/if}
</section>

<style>
  .examples { display: grid; gap: 18px; }
  .intro { display: grid; gap: 10px; }
  h2 { margin: 0; font-size: 18px; }
  p { margin: 2px 0 0; }
  nav { display: flex; gap: 6px; flex-wrap: wrap; }
  nav button.active { border-color: var(--accent); color: var(--accent); background: var(--bg-elev2); }
</style>
