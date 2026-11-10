// Baseline: tight numeric loop (JIT-eligible arithmetic).
let sum: number = 0;
for (let r: number = 0; r < 50; r = r + 1) {
  for (let i: number = 0; i < 1000000; i = i + 1) { sum = sum + (i * 3 - 1); }
}
console.log("compute=" + sum);
