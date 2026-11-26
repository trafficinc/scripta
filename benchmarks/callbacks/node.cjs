'use strict';
const fs = require('node:fs');
const assert = require('node:assert/strict');
const { performance } = require('node:perf_hooks');
const fixture = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const results = [];
let sink;
for (const dataset of fixture.datasets) {
  const { values, rows } = dataset;
  const entries = new Map(dataset.entries);
  const square = v => v * v;
  const makeOffset = amount => v => v + amount;
  const captured = makeOffset(7);
  const even = v => v % 2 === 0;
  const add = (a, v) => a + v;
  const rank = v => v.rank;
  const compareRank = (a, b) => a.rank < b.rank ? -1 : a.rank > b.rank ? 1 : 0;
  // JS Map has no map/filter/reduce methods: these adapters retain typed keys
  // and insertion order without adding an intermediate entries array.
  const mapMap = callback => { const out = new Map(); for (const [k, v] of entries) out.set(k, callback(v, k)); return out; };
  const mapFilter = callback => { const out = new Map(); for (const [k, v] of entries) if (callback(v, k)) out.set(k, v); return out; };
  const mapReduce = (callback, initial) => { let acc = initial; for (const [k, v] of entries) acc = callback(acc, v, k); return acc; };
  const cases = [
    ['list_map', () => values.map(square)],
    ['list_map_capture', () => values.map(captured)],
    ['list_filter', () => values.filter(even)],
    ['list_reduce', () => values.reduce(add, 0)],
    ['map_map', () => mapMap((v, key) => v + 1)],
    ['map_filter', () => mapFilter((v, key) => even(v))],
    ['map_reduce', () => mapReduce((a, v, key) => a + v, 0)],
    ['record_map', () => rows.map(rank)],
    // Cache keys once, sort a fresh decorated array, retain original input.
    ['record_sort_key', () => rows.map(v => ({ key: rank(v), value: v }))
      .sort((a, b) => a.key < b.key ? -1 : a.key > b.key ? 1 : 0).map(v => v.value)],
    ['record_sort_compare', () => rows.slice().sort(compareRank)],
    // Preserve Scripta's collection value semantics explicitly in this case.
    ['reduce_growing_copy', () => values.reduce((a, v) => [...a, v], [])],
    // Additional idiomatic JS baseline; no intermediate accumulator copies.
    ['reduce_growing_mutate_node_only', () => values.reduce((a, v) => { a.push(v); return a; }, [])],
  ];
  const samples = cases.map(() => []);
  const repeats = cases.map(([, run]) => {
    const start = performance.now(); let count = 0;
    do { sink = run(); count++; } while (performance.now() - start < 100);
    return Math.max(1, Math.min(10000, Math.ceil(10 / ((performance.now() - start) / count))));
  });
  for (let sample = 0; sample < 11; sample++) {
    for (let position = 0; position < cases.length; position++) {
      const i = sample % 2 === 0 ? position : cases.length - 1 - position;
      const [name, run] = cases[i];
      const start = performance.now();
      for (let j = 0; j < repeats[i]; j++) sink = run();
      const ms = (performance.now() - start) / repeats[i];
      const expectedName = name === 'reduce_growing_mutate_node_only' ? 'reduce_growing_copy' : name;
      // Canonicalize records exactly as Rust's ordered-map result encoder does.
      const canonical = v => v instanceof Map ? [...v].map(([k, x]) => [k, canonical(x)])
        : Array.isArray(v) ? v.map(canonical)
        : v !== null && typeof v === 'object' ? Object.keys(v).sort().map(k => [k, canonical(v[k])]) : v;
      assert.deepStrictEqual(canonical(sink), dataset.expected[expectedName], name);
      samples[i].push(ms);
    }
  }
  cases.forEach(([name], i) => results.push({ operation: name, size: dataset.size, batch: repeats[i], samples_ms: samples[i] }));
}
console.log(JSON.stringify({runtime: 'node', versions: process.versions, arch: process.arch, execArgv: process.execArgv, results}));
