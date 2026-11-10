// throw / catch in a loop (managed error construction path).
let caught: number = 0;
for (let i: number = 0; i < 300000; i = i + 1) {
  try {
    if (i % 2 == 0) { throw new TypeError("x"); }
  } catch (e) { caught = caught + 1; }
}
console.log("errors=" + caught);
