// Allocation churn to exercise GC marking / minor collections.
class Node { v: number = 0; next: any = null; }
let total: number = 0;
for (let r: number = 0; r < 200; r = r + 1) {
  let head: any = null;
  for (let i: number = 0; i < 20000; i = i + 1) {
    const n = new Node();
    n.v = i;
    n.next = head;
    head = n;
  }
  let p: any = head;
  while (p != null) { total = total + p.v; p = p.next; }
}
console.log("gcchurn=" + total);
