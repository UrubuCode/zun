// Typed class method dispatch + field access in a loop.
class Point {
  x: number = 0;
  y: number = 0;
  constructor(x: number, y: number) { this.x = x; this.y = y; }
  dist(): number { return this.x * this.x + this.y * this.y; }
}
let acc: number = 0;
for (let i: number = 0; i < 2000000; i = i + 1) {
  const p = new Point(i, i + 1);
  acc = acc + p.dist();
}
console.log("methods=" + acc);
