// Dynamic `any` property read/write churn (dynamic member path).
let acc: number = 0;
for (let i: number = 0; i < 1000000; i = i + 1) {
  const o: any = {};
  o.a = i;
  o.b = i + 1;
  acc = acc + o.a + o.b;
}
console.log("dynprops=" + acc);
