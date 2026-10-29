// Web body primitives built locally (no network): Blob sizing and typing, slicing by
// bytes, File metadata, Response status/validation/single-use bodies, and the
// Request body contract.

(async function main(): Promise<void> {
  // ---- Blob sizing: counted in encoded UTF-8 bytes, not code units. ----
  console.log("ascii=" + new Blob(["abc"]).size);
  console.log("latin=" + new Blob(["é"]).size);
  console.log("euro=" + new Blob(["€"]).size);
  console.log("astral=" + new Blob(["\u{1F600}"]).size);
  console.log("lone_surrogate=" + new Blob(["\uD800"]).size);
  console.log("empty=" + new Blob().size + "," + new Blob([]).size + "," + new Blob([""]).size);

  // Parts may be strings, buffers, views and other Blobs, in any mix.
  const mixed = new Blob(["ab", new Uint8Array([67, 68]), new Blob(["ef"]), new ArrayBuffer(2)]);
  console.log("mixed_size=" + mixed.size);
  console.log("mixed_text=" + JSON.stringify(await mixed.text()));
  const fromView = new Uint8Array([1, 2, 3, 4]).subarray(1, 3);
  console.log("view_window=" + new Blob([fromView]).size);
  console.log("dataview_part=" + new Blob([new DataView(new ArrayBuffer(3))]).size);
  console.log("number_part=" + (await new Blob([1234 as any]).text()));
  const typed = new Blob(["xy"], { type: "a/b" });
  console.log("wrapped=" + (await new Blob([typed, "z"]).text()) + " size=" + new Blob([typed, "z"]).size);
  try {
    console.log("non_iterable_size=" + new Blob({} as any).size);
  } catch (e: any) {
    console.log("non_iterable=" + e.constructor.name);
  }
  console.log("string_not_array=" + (function (): string {
    try {
      return String(new Blob("abc" as any).size);
    } catch (e: any) {
      return e.constructor.name;
    }
  })());

  // ---- Blob type: lower-cased when printable ASCII, otherwise kept verbatim. ----
  console.log("type_default=" + JSON.stringify(new Blob(["x"]).type));
  console.log("type_lowered=" + new Blob(["x"], { type: "TEXT/Plain;Charset=UTF-8" }).type);
  console.log("type_spaces=" + JSON.stringify(new Blob(["x"], { type: " text/plain " }).type));
  console.log("type_nonascii=" + JSON.stringify(new Blob(["x"], { type: "tëxt/plain" }).type));

  // ---- slice(): Array-style bounds, a type argument, and byte cuts. ----
  const src = new Blob(["abcdefghij"]);
  console.log("slice_all=" + (await src.slice().text()));
  console.log("slice_from=" + (await src.slice(3).text()));
  console.log("slice_range=" + (await src.slice(2, 5).text()));
  console.log("slice_negative=" + (await src.slice(-3).text()));
  console.log("slice_both_negative=" + (await src.slice(-4, -2).text()));
  console.log("slice_reversed=" + JSON.stringify(await src.slice(5, 2).text()) + " size=" + src.slice(5, 2).size);
  console.log("slice_past_end=" + (await src.slice(8, 99).text()));
  console.log("slice_far_negative=" + (await src.slice(-99, 2).text()));
  console.log("slice_type=" + JSON.stringify(src.slice(0, 1, "X/Y").type) + " parent=" + JSON.stringify(src.type));
  console.log("slice_of_slice=" + (await src.slice(2, 8).slice(1, 3).text()));
  console.log("slice_is_new=" + (src.slice() !== src) + " kind=" + src.slice().constructor.name);
  const multi = new Blob(["€€"]);
  console.log("multi_size=" + multi.size);
  const cut = multi.slice(0, 4);
  console.log("cut_size=" + cut.size);
  const codes: string[] = [];
  for (const ch of await cut.text()) codes.push((ch.codePointAt(0) as number).toString(16));
  console.log("cut_codepoints=" + codes.join(","));

  // ---- Async readers over one Blob, which can be read repeatedly. ----
  const reader = new Blob(["héllo"]);
  console.log("text=" + (await reader.text()));
  const ab = await reader.arrayBuffer();
  console.log("arrayBuffer=" + ab.byteLength + " kind=" + ab.constructor.name + " bytes=" + Array.from(new Uint8Array(ab)).join(","));
  console.log("bytes_method=" + (typeof (reader as any).bytes === "function"));
  const bytes = await (reader as any).bytes();
  console.log("bytes=" + bytes.constructor.name + " " + Array.from(bytes as Uint8Array).join(","));
  console.log("read_twice=" + (await reader.text()) + "|" + (await reader.text()));
  console.log("empty_text=" + JSON.stringify(await new Blob().text()));
  console.log("endings_default=" + JSON.stringify(await new Blob(["a\r\nb\nc"]).text()));
  console.log("endings_transparent=" + JSON.stringify(await new Blob(["a\r\nb\nc"], { endings: "transparent" }).text()));

  // Blob identity.
  console.log("blob_tag=" + Object.prototype.toString.call(new Blob()));
  console.log("blob_ctor=" + new Blob().constructor.name + " instanceof=" + (new Blob() instanceof Blob));
  console.log("size_is_getter=" + (typeof (Object.getOwnPropertyDescriptor(Blob.prototype, "size") as any).get));
  console.log("no_own_size=" + Object.prototype.hasOwnProperty.call(new Blob(), "size"));

  // ---- File: a Blob with a name and lastModified. ----
  const file = new File(["abc"], "x.txt", { lastModified: 1700000000000 });
  console.log("file_name=" + file.name);
  console.log("file_lastModified=" + file.lastModified);
  console.log("file_size=" + file.size + " text=" + (await file.text()));
  console.log("file_is_blob=" + (file instanceof Blob));

  // ---- Response: status validation and the ok window. ----
  const plain = new Response();
  console.log("default=" + plain.status + " ok=" + plain.ok + " type=" + plain.type + " statusText=" + JSON.stringify(plain.statusText) + " bodyUsed=" + plain.bodyUsed);
  console.log("default_body_null=" + (plain.body === null));
  console.log("redirected=" + plain.redirected + " url=" + JSON.stringify(plain.url));
  const statuses: number[] = [199, 200, 204, 299, 300, 399, 400, 599, 600, 1000, 0, -1];
  for (const st of statuses) {
    try {
      const r = new Response(null, { status: st });
      console.log("status[" + st + "]=" + r.status + " ok=" + r.ok);
    } catch (e: any) {
      console.log("status[" + st + "]=" + e.constructor.name);
    }
  }
  console.log("statusText_kept=" + JSON.stringify(new Response(null, { status: 404, statusText: "Nope" }).statusText));

  // Response.error() is a network error: status 0, never ok.
  const err = Response.error();
  console.log("error=" + err.status + " type=" + err.type + " ok=" + err.ok + " bodyUsed=" + err.bodyUsed);

  // Response.redirect() accepts only redirect statuses.
  for (const st of [300, 301, 302, 303, 304, 307, 308, 200, 399]) {
    try {
      const r = Response.redirect("https://example.com/x", st);
      console.log("redirect[" + st + "]=" + r.status + " loc=" + r.headers.get("location"));
    } catch (e: any) {
      console.log("redirect[" + st + "]=" + e.constructor.name);
    }
  }
  console.log("redirect_default=" + Response.redirect("https://example.com/x").status);

  // Response.json() serialises and sets a JSON content type.
  const rj = Response.json({ a: [1, 2], b: null });
  console.log("json_status=" + rj.status + " ct_index=" + (rj.headers.get("content-type") as string).indexOf("application/json"));
  console.log("json_body=" + (await rj.json()).a.join("-"));
  console.log("json_with_status=" + Response.json({}, { status: 418 }).status);
  const circular: any = {};
  circular.self = circular;
  try {
    Response.json(circular);
    console.log("json_circular=accepted");
  } catch (e: any) {
    console.log("json_circular=" + e.constructor.name);
  }

  // ---- A body is single-use; clone() is the only way to read it twice. ----
  const once = new Response("hello");
  console.log("used_before=" + once.bodyUsed + " has_body=" + (once.body !== null));
  console.log("read=" + (await once.text()));
  console.log("used_after=" + once.bodyUsed);
  try {
    await once.text();
    console.log("second_read=accepted");
  } catch (e: any) {
    console.log("second_read=" + e.constructor.name);
  }
  try {
    await once.arrayBuffer();
    console.log("other_reader=accepted");
  } catch (e: any) {
    console.log("other_reader=" + e.constructor.name);
  }
  const original = new Response("cloneable");
  const copy = original.clone();
  console.log("clone_used=" + original.bodyUsed + "," + copy.bodyUsed);
  console.log("clone_a=" + (await original.text()));
  console.log("clone_b=" + (await copy.text()));

  // Body readers over each source shape.
  console.log("from_string=" + (await new Response("abc").text()));
  console.log("from_bytes=" + (await new Response(new Uint8Array([104, 105])).text()));
  console.log("from_buffer=" + (await new Response(new Uint8Array([104, 105]).buffer).text()));
  console.log("from_blob=" + (await new Response(new Blob(["blobbed"])).text()));
  console.log("from_params=" + (await new Response(new URLSearchParams("a=1&b=2")).text()));
  console.log("from_null=" + JSON.stringify(await new Response(null).text()));
  console.log("as_arrayBuffer=" + (await new Response("hi").arrayBuffer()).byteLength);
  const asBlob = await new Response("hi", { headers: { "content-type": "a/b" } }).blob();
  console.log("as_blob=" + asBlob.size + " type=" + asBlob.type);
  console.log("as_bytes=" + Array.from(await (new Response("hi") as any).bytes()).join(","));
  try {
    await new Response("not json").json();
    console.log("bad_json=accepted");
  } catch (e: any) {
    console.log("bad_json=" + e.constructor.name);
  }

  // Passed headers are normalised, and the Headers object is mutable afterwards.
  const withHeaders = new Response("x", { headers: { "X-One": "1", "x-two": "2" } });
  console.log("headers=" + [...withHeaders.headers.keys()].filter(function (k) { return k.indexOf("x-") === 0; }).join(","));
  withHeaders.headers.set("x-three", "3");
  console.log("headers_mutable=" + withHeaders.headers.get("x-three"));
  console.log("response_tag=" + Object.prototype.toString.call(plain));

  // ---- Request: the same body contract, plus method and URL. ----
  const res = new Response(JSON.stringify({ a: 1 }), { headers: { "content-type": "application/json" } });
  console.log("ct=" + res.headers.get("content-type"));
  console.log("txt=" + (await res.text()));
  const rq = new Request("https://example.com/p", { method: "post", body: "payload", headers: { "X-A": "1" } });
  console.log("request=" + rq.method + " " + rq.url + " used=" + rq.bodyUsed + " hdr=" + rq.headers.get("x-a"));
  const rqCopy = rq.clone();
  console.log("request_read=" + (await rq.text()) + " clone=" + (await rqCopy.text()));
  console.log("request_used=" + rq.bodyUsed);
  const simple = new Request("https://example.com/x", { method: "POST", body: "hi" });
  console.log("m=" + simple.method + " u=" + simple.url + " b=" + (await simple.text()));
  console.log("request_from_request=" + new Request(new Request("https://example.com/q", { method: "PUT" })).method);
  try {
    new Request("relative/path");
    console.log("request_relative=accepted");
  } catch (e: any) {
    console.log("request_relative=" + e.constructor.name);
  }
  console.log("request_tag=" + Object.prototype.toString.call(rq));
})();
