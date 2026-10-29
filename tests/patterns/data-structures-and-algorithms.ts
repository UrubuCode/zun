// Classic containers and algorithms, a traced binary heap driving Dijkstra, a
// persistent list with structural sharing, an LRU cache and a bytecode stack
// machine. Output is line-per-case; traces are fully deterministic.

// ---------------------------------------------------------------- containers

class Stack<T> {
  private data: T[] = [];
  push(v: T) { this.data.push(v); }
  pop(): T | undefined { return this.data.pop(); }
  peek(): T | undefined { return this.data[this.data.length - 1]; }
  get size() { return this.data.length; }
  isEmpty() { return this.data.length === 0; }
}
const st = new Stack<number>();
console.log("stack.empty.pop=" + String(st.pop()));
st.push(1);
st.push(2);
st.push(3);
console.log("stack.peek=" + st.peek());
console.log("stack.pop=" + st.pop());
console.log("stack.size=" + st.size);
console.log("stack.isEmpty=" + st.isEmpty());

// Two-stack queue: amortized O(1) dequeue.
class Queue<T> {
  private inbox: T[] = [];
  private outbox: T[] = [];
  enqueue(v: T) { this.inbox.push(v); }
  dequeue(): T | undefined {
    if (this.outbox.length === 0) {
      while (this.inbox.length) this.outbox.push(this.inbox.pop() as T);
    }
    return this.outbox.pop();
  }
  get size() { return this.inbox.length + this.outbox.length; }
}
const q = new Queue<string>();
q.enqueue("a");
q.enqueue("b");
q.enqueue("c");
console.log("queue.dequeue1=" + q.dequeue());
q.enqueue("d");
console.log("queue.dequeue2=" + q.dequeue());
console.log("queue.size=" + q.size);
console.log("queue.drain=" + [q.dequeue(), q.dequeue(), q.dequeue(), q.dequeue()].join(","));
console.log("queue.empty=" + String(q.dequeue()));

class LLNode<T> {
  val: T;
  next: LLNode<T> | null;
  constructor(val: T, next: LLNode<T> | null = null) {
    this.val = val;
    this.next = next;
  }
}
class LinkedList<T> {
  head: LLNode<T> | null = null;
  push(v: T) {
    const node = new LLNode<T>(v);
    if (!this.head) { this.head = node; return; }
    let cur = this.head;
    while (cur.next) cur = cur.next;
    cur.next = node;
  }
  toArray(): T[] {
    const out: T[] = [];
    let cur = this.head;
    while (cur) { out.push(cur.val); cur = cur.next; }
    return out;
  }
  reverse() {
    let prev: LLNode<T> | null = null;
    let cur = this.head;
    while (cur) {
      const next: LLNode<T> | null = cur.next;
      cur.next = prev;
      prev = cur;
      cur = next;
    }
    this.head = prev;
  }
}
const ll = new LinkedList<number>();
for (const v of [1, 2, 3, 4, 5]) ll.push(v);
console.log("list.forward=" + ll.toArray().join(","));
ll.reverse();
console.log("list.reversed=" + ll.toArray().join(","));
const emptyList = new LinkedList<number>();
emptyList.reverse();
console.log("list.empty.reversed=" + emptyList.toArray().length);
const single = new LinkedList<string>();
single.push("solo");
single.reverse();
console.log("list.single.reversed=" + single.toArray().join(","));

// ---------------------------------------------------------------- traced binary heap

// Ordering is (key, name), so ties are total and never depend on sort stability.
type HeapItem = { key: number; name: string };

class TracedHeap {
  private items: HeapItem[] = [];
  readonly trace: string[] = [];

  get size(): number { return this.items.length; }

  private less(a: HeapItem, b: HeapItem): boolean {
    return a.key !== b.key ? a.key < b.key : a.name < b.name;
  }

  push(key: number, name: string): void {
    this.items.push({ key: key, name: name });
    let i = this.items.length - 1;
    let swaps = 0;
    while (i > 0) {
      const parent = (i - 1) >> 1;
      if (!this.less(this.items[i], this.items[parent])) break;
      const t = this.items[i];
      this.items[i] = this.items[parent];
      this.items[parent] = t;
      i = parent;
      swaps += 1;
    }
    this.trace.push("push " + name + "@" + key + " up=" + swaps + " " + this.dump());
  }

  pop(): HeapItem | null {
    if (this.items.length === 0) return null;
    const top = this.items[0];
    const last = this.items.pop() as HeapItem;
    let swaps = 0;
    if (this.items.length > 0) {
      this.items[0] = last;
      let i = 0;
      for (;;) {
        const l = 2 * i + 1;
        const r = l + 1;
        let smallest = i;
        if (l < this.items.length && this.less(this.items[l], this.items[smallest])) smallest = l;
        if (r < this.items.length && this.less(this.items[r], this.items[smallest])) smallest = r;
        if (smallest === i) break;
        const t = this.items[i];
        this.items[i] = this.items[smallest];
        this.items[smallest] = t;
        i = smallest;
        swaps += 1;
      }
    }
    this.trace.push("pop  " + top.name + "@" + top.key + " down=" + swaps + " " + this.dump());
    return top;
  }

  dump(): string {
    return "[" + this.items.map((it) => it.name + ":" + it.key).join(" ") + "]";
  }

  isValid(): boolean {
    for (let i = 1; i < this.items.length; i++) {
      if (this.less(this.items[i], this.items[(i - 1) >> 1])) return false;
    }
    return true;
  }
}

const heap = new TracedHeap();
const seq = [5, 3, 8, 1, 9, 2, 7, 4, 6, 0];
for (const v of seq) heap.push(v, "n" + v);
for (const line of heap.trace) console.log("heap.build=" + line);
console.log("heap.valid_after_build=" + heap.isValid());
const popped: number[] = [];
while (heap.size > 0) popped.push((heap.pop() as HeapItem).key);
console.log("heap.sorted=" + popped.join(","));
for (const line of heap.trace.slice(seq.length)) console.log("heap.drain=" + line);
console.log("heap.pop.empty=" + String(heap.pop()));

const tied = new TracedHeap();
for (const name of ["d", "a", "c", "b"]) tied.push(1, name);
const tieOrder: string[] = [];
while (tied.size > 0) tieOrder.push((tied.pop() as HeapItem).name);
console.log("heap.tie_order=" + tieOrder.join(","));

// ---------------------------------------------------------------- sorting and searching

function binarySearch(arr: number[], target: number): number {
  let lo = 0;
  let hi = arr.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (arr[mid] === target) return mid;
    if (arr[mid] < target) lo = mid + 1;
    else hi = mid - 1;
  }
  return -1;
}
const sortedArr = [1, 3, 5, 7, 9, 11, 13, 15, 17, 19];
console.log("bsearch.7=" + binarySearch(sortedArr, 7));
console.log("bsearch.1=" + binarySearch(sortedArr, 1));
console.log("bsearch.19=" + binarySearch(sortedArr, 19));
console.log("bsearch.6=" + binarySearch(sortedArr, 6));
console.log("bsearch.empty=" + binarySearch([], 3));
console.log("bsearch.below=" + binarySearch(sortedArr, -100));

function quicksort(arr: number[], lo = 0, hi = arr.length - 1): void {
  if (lo >= hi) return;
  const pivot = arr[hi];
  let i = lo;
  for (let j = lo; j < hi; j++) {
    if (arr[j] <= pivot) {
      [arr[i], arr[j]] = [arr[j], arr[i]];
      i++;
    }
  }
  [arr[i], arr[hi]] = [arr[hi], arr[i]];
  quicksort(arr, lo, i - 1);
  quicksort(arr, i + 1, hi);
}
const qs = [5, 3, 8, 1, 9, 2, 7, 4, 6];
quicksort(qs);
console.log("qsort.sorted=" + qs.join(","));
const dups = [3, 1, 3, 2, 1];
quicksort(dups);
console.log("qsort.dups=" + dups.join(","));
const alreadySorted = [1, 2, 3, 4];
quicksort(alreadySorted);
console.log("qsort.presorted=" + alreadySorted.join(","));
const emptyArr: number[] = [];
quicksort(emptyArr);
console.log("qsort.empty=" + emptyArr.length);

const heapSorted = new TracedHeap();
for (const v of [9, 4, 4, 0, -2]) heapSorted.push(v, "h" + v + "_" + heapSorted.size);
const hs: number[] = [];
while (heapSorted.size > 0) hs.push((heapSorted.pop() as HeapItem).key);
console.log("heapsort.negatives=" + hs.join(","));

// ---------------------------------------------------------------- graphs

const graph = new Map<number, number[]>([
  [1, [2, 3]], [2, [4, 5]], [3, [6]], [4, []], [5, []], [6, []],
]);
function bfs(g: Map<number, number[]>, start: number): number[] {
  const visited = new Set<number>([start]);
  const queueArr = [start];
  const order: number[] = [];
  while (queueArr.length) {
    const node = queueArr.shift() as number;
    order.push(node);
    for (const nb of g.get(node) ?? []) {
      if (!visited.has(nb)) {
        visited.add(nb);
        queueArr.push(nb);
      }
    }
  }
  return order;
}
function dfs(g: Map<number, number[]>, start: number): number[] {
  const visited = new Set<number>();
  const stack = [start];
  const order: number[] = [];
  while (stack.length) {
    const node = stack.pop() as number;
    if (visited.has(node)) continue;
    visited.add(node);
    order.push(node);
    const neighbors = [...(g.get(node) ?? [])].reverse();
    for (const nb of neighbors) if (!visited.has(nb)) stack.push(nb);
  }
  return order;
}
console.log("bfs.1=" + bfs(graph, 1).join(","));
console.log("dfs.1=" + dfs(graph, 1).join(","));
console.log("bfs.leaf=" + bfs(graph, 6).join(","));
console.log("bfs.missing=" + bfs(graph, 99).join(","));

function lcs(a: string, b: string): number {
  const m = a.length;
  const n = b.length;
  const dp: number[][] = Array.from({ length: m + 1 }, () => new Array<number>(n + 1).fill(0));
  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      dp[i][j] = a[i - 1] === b[j - 1] ? dp[i - 1][j - 1] + 1 : Math.max(dp[i - 1][j], dp[i][j - 1]);
    }
  }
  return dp[m][n];
}
console.log("lcs.classic=" + lcs("ABCBDAB", "BDCAB"));
console.log("lcs.empty=" + lcs("", "abc"));
console.log("lcs.same=" + lcs("kitten", "kitten"));

// ---------------------------------------------------------------- Dijkstra over the traced heap

const edges: Array<[string, string, number]> = [
  ["A", "B", 4], ["A", "C", 2],
  ["B", "C", 5], ["B", "D", 10],
  ["C", "E", 3],
  ["D", "F", 11], ["E", "D", 4],
  ["B", "E", 1], ["F", "G", 2], ["D", "G", 5],
  ["H", "G", 1],
];
const adj: Record<string, Array<[string, number]>> = {};
for (const [u, v, w] of edges) {
  if (adj[u] === undefined) adj[u] = [];
  if (adj[v] === undefined) adj[v] = [];
  adj[u].push([v, w]);
  adj[v].push([u, w]);
}
for (const k of Object.keys(adj)) {
  adj[k].sort((x, y) => (x[0] < y[0] ? -1 : x[0] > y[0] ? 1 : 0));
}
console.log("graph.nodes=" + Object.keys(adj).sort().join(","));
console.log("graph.degrees=" + Object.keys(adj).sort().map((k) => k + ":" + adj[k].length).join(","));

function dijkstra(start: string): string[] {
  const dist: Record<string, number> = {};
  const prev: Record<string, string> = {};
  const done: Record<string, boolean> = {};
  const log: string[] = [];
  for (const k of Object.keys(adj)) dist[k] = Infinity;
  dist[start] = 0;

  const pq = new TracedHeap();
  pq.push(0, start);

  while (pq.size > 0) {
    const top = pq.pop() as HeapItem;
    if (done[top.name]) {
      log.push("skip " + top.name + "@" + top.key);
      continue;
    }
    done[top.name] = true;
    log.push("settle " + top.name + "=" + top.key);
    for (const [next, w] of adj[top.name]) {
      if (done[next]) continue;
      const candidate = dist[top.name] + w;
      if (candidate < dist[next]) {
        log.push("  relax " + top.name + "->" + next + " " + (dist[next] === Infinity ? "inf" : String(dist[next])) + "=>" + candidate);
        dist[next] = candidate;
        prev[next] = top.name;
        pq.push(candidate, next);
      } else {
        log.push("  keep  " + top.name + "->" + next + " " + candidate + ">=" + dist[next]);
      }
    }
  }
  for (const line of pq.trace) log.push("  heap " + line);

  log.push("dist " + Object.keys(dist).sort().map((k) => k + "=" + (dist[k] === Infinity ? "inf" : String(dist[k]))).join(","));
  const path = (to: string): string => {
    const chain: string[] = [];
    let cur: string | undefined = to;
    while (cur !== undefined) {
      chain.push(cur);
      cur = prev[cur];
    }
    return chain.reverse().join(">");
  };
  for (const k of Object.keys(dist).sort()) log.push("path " + k + " = " + path(k));
  return log;
}

for (const line of dijkstra("A")) console.log("dijkstra.A=" + line);
for (const line of dijkstra("G")) console.log("dijkstra.G=" + line);

adj["Z"] = [];
const fromA = dijkstra("A");
console.log("dijkstra.isolated=" + fromA[fromA.length - 1]);
console.log("dijkstra.isolated.dist=" + fromA.filter((l) => l.startsWith("dist ")).join(""));

// ---------------------------------------------------------------- persistent list

interface Cell { head: number; tail: Cell | null; }
const NIL: Cell | null = null;

function cons(head: number, tail: Cell | null): Cell {
  return Object.freeze({ head: head, tail: tail }) as Cell;
}
function fromArray(xs: number[]): Cell | null {
  let out: Cell | null = NIL;
  for (let i = xs.length - 1; i >= 0; i--) out = cons(xs[i], out);
  return out;
}
function toArray(list: Cell | null): number[] {
  const out: number[] = [];
  let cur = list;
  while (cur !== null) {
    out.push(cur.head);
    cur = cur.tail;
  }
  return out;
}
function len(list: Cell | null): number {
  let n = 0;
  let cur = list;
  while (cur !== null) {
    n += 1;
    cur = cur.tail;
  }
  return n;
}
function nth(list: Cell | null, i: number): Cell | null {
  let cur = list;
  while (cur !== null && i > 0) {
    cur = cur.tail;
    i -= 1;
  }
  return cur;
}
function sharedSuffix(a: Cell | null, b: Cell | null): number {
  const seen = new Set<Cell>();
  let cur = a;
  while (cur !== null) {
    seen.add(cur);
    cur = cur.tail;
  }
  let count = 0;
  cur = b;
  while (cur !== null) {
    if (seen.has(cur)) count += 1;
    cur = cur.tail;
  }
  return count;
}
function setAt(list: Cell | null, index: number, value: number): Cell | null {
  if (list === null) return null;
  if (index === 0) return cons(value, list.tail);
  return cons(list.head, setAt(list.tail, index - 1, value));
}
function append(a: Cell | null, b: Cell | null): Cell | null {
  if (a === null) return b;
  return cons(a.head, append(a.tail, b));
}
function reverseList(list: Cell | null): Cell | null {
  let out: Cell | null = NIL;
  let cur = list;
  while (cur !== null) {
    out = cons(cur.head, out);
    cur = cur.tail;
  }
  return out;
}
function equal(a: Cell | null, b: Cell | null): boolean {
  while (a !== null && b !== null) {
    if (a === b) return true;
    if (a.head !== b.head) return false;
    a = a.tail;
    b = b.tail;
  }
  return a === b;
}

const base = fromArray([1, 2, 3, 4, 5]) as Cell;
console.log("plist.base=" + toArray(base).join(","));
console.log("plist.len=" + len(base));

const prepended = cons(0, base);
console.log("plist.prepended=" + toArray(prepended).join(","));
console.log("plist.base_after_prepend=" + toArray(base).join(","));
console.log("plist.tail_is_shared=" + (prepended.tail === base));

const replaced = setAt(base, 2, 99) as Cell;
console.log("plist.replaced=" + toArray(replaced).join(","));
console.log("plist.base_after_set=" + toArray(base).join(","));
console.log("plist.copied_0=" + (nth(replaced, 0) !== nth(base, 0)));
console.log("plist.copied_1=" + (nth(replaced, 1) !== nth(base, 1)));
console.log("plist.copied_2=" + (nth(replaced, 2) !== nth(base, 2)));
console.log("plist.shared_3=" + (nth(replaced, 3) === nth(base, 3)));
console.log("plist.shared_4=" + (nth(replaced, 4) === nth(base, 4)));
console.log("plist.shared_after_set2=" + sharedSuffix(base, replaced));
console.log("plist.shared_after_set0=" + sharedSuffix(base, setAt(base, 0, 42)));
console.log("plist.shared_after_set4=" + sharedSuffix(base, setAt(base, 4, 42)));

// Frozen cells refuse mutation, so sharing is safe. Reflect reports refusal as booleans.
console.log("plist.is_frozen=" + Object.isFrozen(base));
console.log("plist.set_head_refused=" + Reflect.set(base, "head", 1000));
console.log("plist.set_tail_refused=" + Reflect.set(base, "tail", null));
console.log("plist.delete_head_refused=" + Reflect.deleteProperty(base, "head"));
console.log("plist.head_unchanged=" + (base.head === 1));

const history: Array<Cell | null> = [base];
for (let i = 0; i < 5; i++) history.push(setAt(history[history.length - 1], i, (i + 1) * 10));
for (let i = 0; i < history.length; i++) {
  console.log("plist.v" + i + "=" + toArray(history[i]).join(",") + " shared_with_v0=" + sharedSuffix(base, history[i]));
}

const reversed = reverseList(base);
console.log("plist.reversed=" + toArray(reversed).join(","));
console.log("plist.reversed_shares=" + sharedSuffix(base, reversed));

const second = fromArray([7, 8]) as Cell;
const joined = append(base, second) as Cell;
console.log("plist.joined=" + toArray(joined).join(","));
console.log("plist.joined_shares_second=" + (nth(joined, 5) === second));
console.log("plist.joined_shares_first=" + sharedSuffix(base, joined));

console.log("plist.eq_same=" + equal(base, base));
console.log("plist.eq_copy=" + equal(base, fromArray([1, 2, 3, 4, 5])));
console.log("plist.eq_diff=" + equal(base, fromArray([1, 2, 3, 4, 6])));
console.log("plist.eq_shorter=" + equal(base, fromArray([1, 2, 3])));
console.log("plist.eq_empty=" + equal(NIL, NIL));

// A deep chain holds every version without a stack problem.
let deep: Cell | null = NIL;
const versions: Array<Cell | null> = [];
for (let i = 0; i < 200; i++) {
  deep = cons(i, deep);
  versions.push(deep);
}
console.log("plist.deep_len=" + len(deep));
console.log("plist.v10_len=" + len(versions[10]));
console.log("plist.v199_head=" + (versions[199] as Cell).head);
console.log("plist.v0_len=" + len(versions[0]));
console.log("plist.v100_shares=" + (versions[100] === nth(deep, 99)));

// Persistent association list: an update shadows the older binding.
interface Entry { key: string; value: number; next: Entry | null; }
function put(map: Entry | null, key: string, value: number): Entry {
  return Object.freeze({ key: key, value: value, next: map }) as Entry;
}
function getEntry(map: Entry | null, key: string): number | undefined {
  let cur = map;
  while (cur !== null) {
    if (cur.key === key) return cur.value;
    cur = cur.next;
  }
  return undefined;
}
function keysOf(map: Entry | null): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  let cur = map;
  while (cur !== null) {
    if (!seen.has(cur.key)) {
      seen.add(cur.key);
      out.push(cur.key);
    }
    cur = cur.next;
  }
  return out.sort();
}
const m0 = put(put(put(null, "a", 1), "b", 2), "c", 3);
const m1 = put(m0, "b", 20);
const m2 = put(m1, "d", 4);
console.log("assoc.m0=" + keysOf(m0).map((k) => k + "=" + getEntry(m0, k)).join(","));
console.log("assoc.m1=" + keysOf(m1).map((k) => k + "=" + getEntry(m1, k)).join(","));
console.log("assoc.m2=" + keysOf(m2).map((k) => k + "=" + getEntry(m2, k)).join(","));
console.log("assoc.m0_b_still=" + getEntry(m0, "b"));
console.log("assoc.m1_shares_m0=" + (m1.next === m0));
console.log("assoc.m2_shares_m1=" + (m2.next === m1));
console.log("assoc.missing=" + String(getEntry(m2, "zz")));

// ---------------------------------------------------------------- LRU cache

class Lru {
  limit: number;
  map = new Map<string, number>();
  constructor(limit: number) { this.limit = limit; }
  get(k: string): number {
    if (!this.map.has(k)) return -1;
    const v = this.map.get(k) as number;
    this.map.delete(k);
    this.map.set(k, v);
    return v;
  }
  set(k: string, v: number) {
    if (this.map.has(k)) this.map.delete(k);
    this.map.set(k, v);
    if (this.map.size > this.limit) {
      const oldest = this.map.keys().next().value as string;
      this.map.delete(oldest);
    }
  }
}
const lru = new Lru(3);
lru.set("a", 1);
lru.set("b", 2);
lru.set("c", 3);
console.log("lru.get_a=" + lru.get("a"));
lru.set("d", 4);
console.log("lru.order=" + [...lru.map.entries()].map(([k, v]) => k + v).join(","));
console.log("lru.get_b=" + lru.get("b"));
lru.set("c", 30);
console.log("lru.reinsert=" + [...lru.map.entries()].map(([k, v]) => k + v).join(","));
console.log("lru.get_missing=" + lru.get("zz"));

// ---------------------------------------------------------------- bytecode stack machine

const OP_PUSH = 0;
const OP_LOAD = 1;
const OP_STORE = 2;
const OP_ADD = 3;
const OP_SUB = 4;
const OP_MUL = 5;
const OP_LT = 6;
const OP_JMP = 7;
const OP_JZ = 8;
const OP_CALL = 9;
const OP_RET = 10;
const OP_PRINT = 11;
const OP_HALT = 12;
const OP_DUP = 13;
const OP_POP = 14;

const NAMES = ["PUSH", "LOAD", "STORE", "ADD", "SUB", "MUL", "LT", "JMP", "JZ", "CALL", "RET", "PRINT", "HALT", "DUP", "POP"];

interface Frame {
  ret: number;
  locals: number[];
  base: number;
}

function runVm(code: number[], entry: number, traceLimit: number): string[] {
  const out: string[] = [];
  const stack: number[] = [];
  const frames: Frame[] = [{ ret: -1, locals: [0, 0, 0, 0], base: 0 }];
  let pc = entry;
  let steps = 0;

  for (;;) {
    if (steps >= 5000) {
      out.push("ABORT");
      break;
    }
    const op = code[pc];
    const frame = frames[frames.length - 1];
    if (steps < traceLimit) {
      out.push(steps + " pc=" + pc + " " + NAMES[op] + " stack=[" + stack.join(",") + "]");
    }
    pc += 1;
    steps += 1;

    switch (op) {
      case OP_PUSH: stack.push(code[pc]); pc += 1; break;
      case OP_LOAD: stack.push(frame.locals[code[pc]]); pc += 1; break;
      case OP_STORE: frame.locals[code[pc]] = stack.pop() as number; pc += 1; break;
      case OP_ADD: { const b = stack.pop() as number; stack.push((stack.pop() as number) + b); break; }
      case OP_SUB: { const b = stack.pop() as number; stack.push((stack.pop() as number) - b); break; }
      case OP_MUL: { const b = stack.pop() as number; stack.push((stack.pop() as number) * b); break; }
      case OP_LT: { const b = stack.pop() as number; stack.push((stack.pop() as number) < b ? 1 : 0); break; }
      case OP_JMP: pc = code[pc]; break;
      case OP_JZ: {
        const t = code[pc];
        pc += 1;
        if ((stack.pop() as number) === 0) pc = t;
        break;
      }
      case OP_CALL: {
        const target = code[pc];
        const argc = code[pc + 1];
        pc += 2;
        const locals = [0, 0, 0, 0];
        for (let i = argc - 1; i >= 0; i--) locals[i] = stack.pop() as number;
        frames.push({ ret: pc, locals: locals, base: stack.length });
        pc = target;
        break;
      }
      case OP_RET: {
        const value = stack.pop() as number;
        const done = frames.pop() as Frame;
        stack.length = done.base;
        stack.push(value);
        pc = done.ret;
        break;
      }
      case OP_PRINT: out.push("  out " + stack[stack.length - 1]); break;
      case OP_DUP: stack.push(stack[stack.length - 1]); break;
      case OP_POP: stack.pop(); break;
      case OP_HALT:
        out.push("HALT top=" + (stack.length === 0 ? "empty" : String(stack[stack.length - 1])) + " steps=" + steps);
        return out;
      default:
        out.push("BAD op " + op);
        return out;
    }
  }
  return out;
}

const arith = [OP_PUSH, 3, OP_PUSH, 4, OP_ADD, OP_PUSH, 5, OP_MUL, OP_PRINT, OP_HALT];
for (const line of runVm(arith, 0, 20)) console.log("vm.arith=" + line);

// Counted loop: local 0 counts 1..5, local 1 accumulates the sum.
const loop = [
  OP_PUSH, 0, OP_STORE, 1,        // 0-3   sum = 0
  OP_PUSH, 1, OP_STORE, 0,        // 4-7   i = 1
  OP_LOAD, 0, OP_PUSH, 6, OP_LT,  // 8-12  i < 6
  OP_JZ, 31,                      // 13-14 exit to 31 when false
  OP_LOAD, 1, OP_LOAD, 0, OP_ADD, OP_STORE, 1,   // 15-21 sum += i
  OP_LOAD, 0, OP_PUSH, 1, OP_ADD, OP_STORE, 0,   // 22-28 i += 1
  OP_JMP, 8,                      // 29-30
  OP_LOAD, 1, OP_PRINT, OP_HALT,  // 31-34
];
for (const line of runVm(loop, 0, 12)) console.log("vm.loop=" + line);

// Recursive factorial through CALL / RET; fact(n) = n < 2 ? 1 : n * fact(n - 1).
const FACT = 12;
const factorialProgram = [
  OP_PUSH, 5,
  OP_CALL, FACT, 1,
  OP_PRINT,
  OP_HALT,
  OP_HALT, OP_HALT, OP_HALT, OP_HALT, OP_HALT,
  OP_LOAD, 0, OP_PUSH, 2, OP_LT,
  OP_JZ, 22,
  OP_PUSH, 1, OP_RET,
  OP_LOAD, 0,
  OP_LOAD, 0, OP_PUSH, 1, OP_SUB,
  OP_CALL, FACT, 1,
  OP_MUL, OP_RET,
];
for (const line of runVm(factorialProgram, 0, 14)) console.log("vm.fact=" + line);

// Nested calls: double(triple(2)).
const DOUBLE = 10;
const TRIPLE = 16;
const nested = [
  OP_PUSH, 2,
  OP_CALL, TRIPLE, 1,
  OP_CALL, DOUBLE, 1,
  OP_PRINT, OP_HALT,
  OP_LOAD, 0, OP_PUSH, 2, OP_MUL, OP_RET,
  OP_LOAD, 0, OP_PUSH, 3, OP_MUL, OP_RET,
];
for (const line of runVm(nested, 0, 30)) console.log("vm.nested=" + line);

// An unknown opcode is reported rather than thrown.
for (const line of runVm([OP_PUSH, 1, 99, OP_HALT], 0, 5)) console.log("vm.bad=" + line);

// DUP and POP, plus a conditional jump that is taken.
const stackOps = [
  OP_PUSH, 7, OP_DUP, OP_ADD, OP_DUP, OP_PRINT,
  OP_PUSH, 0, OP_JZ, 12,
  OP_POP, OP_HALT,
  OP_PUSH, 100, OP_ADD, OP_PRINT, OP_HALT,
];
for (const line of runVm(stackOps, 0, 20)) console.log("vm.dupcase=" + line);
