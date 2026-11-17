// Closures created inside callbacks capture the callback's own bindings, and a
// closure inside a declaration's initializer may refer to the declared name.
function later(cb: () => void): void {
  setTimeout(() => cb(), 1);
}
function outer(cb: (x: any) => void): void {
  cb(1);
}
function get(url: string, cb: (res: any) => void): any {
  setTimeout(() => cb({ s: url }), 1);
  return { id: url };
}

outer((x: any) => {
  const r: any = { n: x };
  later(() => console.log("nested", r.n));
});

function selfReference() {
  const req = get("u", (res: any) => {
    console.log("self", req.id, res.s);
    later(() => console.log("self-nested", req.id));
  });
}
selfReference();
