// WHATWG URL: parsing, normalisation, components, origin, relative resolution,
// percent-encoding, canParse, invalid input and the setters (including the
// assignments that are silently ignored instead of throwing).

function describe(input: string, base?: string): string {
  const label = JSON.stringify(input) + (base !== undefined ? " @" + JSON.stringify(base) : "");
  try {
    const u = base !== undefined ? new URL(input, base) : new URL(input);
    return label + " -> href=" + u.href + " path=" + JSON.stringify(u.pathname);
  } catch (e: any) {
    return label + " -> " + e.name;
  }
}

function components(input: string): void {
  try {
    const u = new URL(input);
    console.log(
      JSON.stringify(input) +
        " protocol=" + u.protocol +
        " host=" + JSON.stringify(u.host) +
        " hostname=" + JSON.stringify(u.hostname) +
        " port=" + JSON.stringify(u.port) +
        " pathname=" + JSON.stringify(u.pathname) +
        " search=" + JSON.stringify(u.search) +
        " hash=" + JSON.stringify(u.hash) +
        " origin=" + u.origin +
        " href=" + u.href,
    );
  } catch (e: any) {
    console.log(JSON.stringify(input) + " -> " + e.name);
  }
}

// --- Full component split on a representative URL --------------------------
components("https://user:pass@example.com:8443/a/b/?q=1&r=2#frag");
components("http://example.com");
components("https://example.com/a?#");
components("https://example.com/a#");

// --- Default ports disappear from host and href ----------------------------
components("https://example.com:443/a");
components("http://example.com:80/a");
components("ws://example.com:80/a");
components("wss://example.com:443/a");
components("ftp://example.com:21/a");
components("https://example.com:80/a");
components("http://example.com:443/a");
components("http://example.com:0/a");
components("http://example.com:65535/a");
components("http://example.com:65536/a");
components("http://example.com:/a");
components("non-special://example.com:80/a");
components("https://example.com:1234/");

// --- Host case folding, trailing dot, IPv4 and IPv6 normalisation -----------
components("https://EXAMPLE.COM/A");
components("HTTPS://ExAmPlE.cOm./a");
components("https://192.168.000.001/");
components("https://0x7f.1/");
components("https://0177.0.0.1/");
components("https://2130706433/");
components("https://1.1/");
components("https://999.1.1.1/");
components("https://1.2.3.4.5/");
components("https://[2001:DB8:0:0:0:0:0:1]:8080/p");
components("https://[::1]/");
components("https://[::ffff:1.2.3.4]/");
components("https://[0:0:0:0:0:0:0:0]/");
components("https://[::1]:443/");
components("https://[:::1]/");
components("https://[::1/");
components("https://xn--bcher-kva.example/");
components("https://%65xample.com/");
components("https://exa%20mple.com/");

// --- Backslashes, tabs, newlines and surrounding whitespace ----------------
components("https:\\\\example.com\\a\\b");
components("https://example.com\\a");
components("non-special:\\\\example.com\\a");
components("ht\ttps://exa\nmple.com/a\rb");
components("https://example.com/a\tb?c\nd#e\rf");
console.log("trim_spaces=" + JSON.stringify(new URL("   https://example.com/x   ").href));
console.log("trim_controls=" + JSON.stringify(new URL("\u0001https://example.com/x\u0001").href));
console.log("strip_in_search=" + new URL("https://example.com/?a=1\t2").search);

// --- Userinfo is kept, percent-encoded, and never leaks into host/origin ----
const withUser = new URL("https://us er:p@ss@example.com:8443/p");
console.log("user=" + withUser.username + " pass=" + withUser.password + " host=" + withUser.host + " origin=" + withUser.origin);
console.log("user_href=" + withUser.href);
console.log("empty_user=" + new URL("https://@example.com/").href);
console.log("only_colon=" + new URL("https://:@example.com/").href);

// --- Scheme shapes: special vs opaque, file:, data:, blob:, about: ----------
components("file:///c:/dir/file.txt");
components("file://host/share");
components("file:///");
components("file://localhost/c:/x");
components("file:///C|/x");
components("file:///./x");
components("mailto:user@example.com?subject=hi");
components("data:text/plain,hello");
components("data:;base64,aGk=");
components("about:blank");
components("blob:https://example.com/1234");
components("blob:null/1234");
components("javascript:void(0)");
components("non-special://host/path");
components("non-special:/path");
components("non-special:path");
components("urn:isbn:0451450523");
components("tel:+15551234");

// --- Origin follows the scheme: real origin for special/blob/https, opaque else
for (const s of ["foo://host/..", "blob:https://a.com/u", "blob:null/x", "https://a.com:8443/x", "http://a.com:80/", "file:///x", "data:text/plain,hi", "ws://h/p", "mailto:a@b.com"]) {
  console.log("origin[" + s + "]=" + new URL(s).origin);
}

// --- A special scheme requires a host; an opaque one does not ---------------
components("https:///a");
components("https://");
components("http://?q=1");
components("non-special://");

// --- Dot segments are removed at parse time ---------------------------------
components("http://h/a/b/../c");
components("http://h/a/b/./c");
components("http://h/a/../../c");
components("http://h/../c");
components("http://h/a/b/..");
components("http://h/a/b/.");
components("http://h/a/b/../");
components("http://h/./././a");
components("http://h/a/%2e%2e/b");
components("http://h/a/%2E/b");
components("http://h/a/...");
components("http://h/a/....");

// --- Backslash is a separator for special schemes, literal otherwise --------
components("http://h/a\\b\\c");
components("http:\\\\h\\a");
components("mailto:a\\b");
components("foo://h/a\\b");

// --- Empty path: "/" on special schemes, kept as-is on opaque ones ----------
components("http://h");
components("http://h?q");
components("http://h#f");
components("foo://h");
components("foo:/a");
components("foo:a");

// --- Percent-encoding: path set, invalid escapes kept, case preserved -------
components("http://h/a b");
components("http://h/a%2fb");
components("http://h/a%zz");
components("http://h/\u00e9");
components("https://x.test/a/%zz/%E0%A4%A?x=%zz");
components("http://h/a?b c#d e");
components("https://example.com/a?b=\u00e9\u20ac#\u{1F600}");

// --- Relative resolution against a base -------------------------------------
const base = "https://example.com/a/b/c?x=1#f";
for (const r of ["d", "./d", "../d", "../../d", "../../../d", "/d", "//other.example/d", "?q=2", "#g", "", "http://elsewhere.example/z", "../d/./e?space=a b&plus=a+b#frag"]) {
  console.log("rel[" + JSON.stringify(r) + "]=" + new URL(r, base).href);
}
console.log(describe("../x", "http://h/a/b/c"));
console.log(describe("./x", "http://h/a/b/c"));
console.log(describe("/x", "http://h/a/b/c"));
console.log(describe("x", "http://h/a/b/c"));
console.log(describe("", "http://h/a/b/c"));
console.log(describe("?q", "http://h/a/b/c?old#frag"));
console.log(describe("#f", "http://h/a/b/c?old#frag"));
console.log(describe("//other/x", "http://h/a/b/c"));
console.log(describe("https://other/x", "http://h/a/b/c"));
console.log(describe("..", "http://h/a/b/"));
console.log(describe("d", "mailto:a@b.com"));
console.log("opaque_base_fragment=" + new URL("#f", "mailto:a@b.com").href);
console.log("base_as_url_object=" + new URL("d", new URL(base) as any).href);

// --- Invalid input throws TypeError; a space in a path is accepted and encoded ---
const refused: string[] = ["", "   ", "//example.com/a", "/just/a/path", "example.com", "https://[bad", "https://exa mple.com/", "http://%zz.example/", "nope", "http://", "https://example.com/a b", "http://[:::1]"];
for (const s of refused) {
  try {
    const u = new URL(s);
    console.log("refused[" + JSON.stringify(s) + "]=ok:" + u.href);
  } catch (e: any) {
    console.log("refused[" + JSON.stringify(s) + "]=" + e.name + " instanceofTypeError=" + (e instanceof TypeError));
  }
}
try {
  new URL("/x", "nope");
  console.log("bad_base=no-throw");
} catch (e: any) {
  console.log("bad_base=" + e.name + " instanceofTypeError=" + (e instanceof TypeError));
}

// --- URL.canParse and URL.parse mirror the constructor ----------------------
const probes: Array<[string, string | undefined]> = [
  ["https://x.example", undefined],
  ["/rel", undefined],
  ["/rel", "https://x.example"],
  ["http://[bad", undefined],
  ["", "https://x.example"],
  ["", undefined],
  ["d", "mailto:a@b.com"],
  ["nope", undefined],
];
for (const p of probes) {
  const ok = p[1] === undefined ? URL.canParse(p[0]) : URL.canParse(p[0], p[1]);
  let ctor = "ok";
  try {
    if (p[1] === undefined) new URL(p[0]);
    else new URL(p[0], p[1]);
  } catch (e: any) {
    ctor = "throw";
  }
  console.log("canParse[" + JSON.stringify(p[0]) + "," + String(p[1]) + "]=" + ok + " ctor=" + ctor);
}
console.log("parse_bad=" + URL.parse("nope"));
console.log("parse_good=" + URL.parse("HTTP://EX.com/x")?.href);

// --- toString / toJSON / JSON / brand --------------------------------------
const u = new URL("https://example.com/p?q=1#f");
console.log("toString=" + (u.toString() === u.href) + " toJSON=" + (u.toJSON() === u.href) + " json=" + JSON.stringify({ u: u }));
console.log("tag=" + Object.prototype.toString.call(u));
console.log("href_is_accessor=" + typeof (Object.getOwnPropertyDescriptor(URL.prototype, "href") as any).get);

// --- Setter: protocol --------------------------------------------------------
const fresh = function (): URL {
  return new URL("https://user:pass@example.com:8443/p/q?a=1#h");
};
const proto = fresh();
proto.protocol = "http:";
console.log("proto_special_swap=" + proto.protocol + " port=" + proto.port);
proto.protocol = "ftp:";
console.log("proto_to_ftp=" + proto.protocol);
proto.protocol = "mailto:";
console.log("proto_to_opaque=" + proto.protocol);
proto.protocol = "wss";
console.log("proto_no_colon=" + proto.protocol);
proto.protocol = "";
console.log("proto_empty=" + proto.protocol);
proto.protocol = "1nvalid:";
console.log("proto_invalid=" + proto.protocol);
proto.protocol = "HTTPS:";
console.log("proto_uppercase=" + proto.protocol);
const swap = new URL("http://example.com:443/");
console.log("before_swap=" + swap.href);
swap.protocol = "https:";
console.log("after_swap=" + swap.href + " port=" + JSON.stringify(swap.port));

// --- Setter: port ----------------------------------------------------------
const port = fresh();
port.port = "1234";
console.log("port_set=" + port.port + " host=" + port.host);
port.port = "443";
console.log("port_default=" + JSON.stringify(port.port));
port.port = "65536";
console.log("port_over=" + JSON.stringify(port.port));
port.port = "abc";
console.log("port_text=" + JSON.stringify(port.port));
port.port = "12abc";
console.log("port_prefix=" + JSON.stringify(port.port));
port.port = "-1";
console.log("port_negative=" + JSON.stringify(port.port));
port.port = "";
console.log("port_empty=" + JSON.stringify(port.port));
port.port = 99 as any;
console.log("port_number=" + JSON.stringify(port.port));

// --- Setter: hostname and host ---------------------------------------------
const host = fresh();
host.hostname = "other.example";
console.log("hostname_set=" + host.host);
host.hostname = "";
console.log("hostname_empty_ignored=" + host.hostname);
host.hostname = "third.example:99";
console.log("hostname_with_port=" + host.host);
host.host = "fourth.example:1234";
console.log("host_with_port=" + host.host + " port=" + host.port);
host.host = "fifth.example";
console.log("host_without_port_keeps=" + host.host + " port=" + host.port);
host.host = "";
console.log("host_empty_ignored=" + host.host);
host.hostname = "UPPER.example";
console.log("hostname_lowercased=" + host.hostname);
host.hostname = "has space";
console.log("hostname_space_ignored=" + host.hostname);

// --- Setter: pathname -------------------------------------------------------
const path = fresh();
path.pathname = "a b/c";
console.log("path_encoded=" + path.pathname);
path.pathname = "/x/../y/./z";
console.log("path_dotted=" + path.pathname);
path.pathname = "";
console.log("path_empty=" + JSON.stringify(path.pathname));
path.pathname = "no-slash";
console.log("path_slash_added=" + path.pathname);
path.pathname = "back\\slash";
console.log("path_backslash=" + path.pathname);
path.pathname = "q?x#y";
console.log("path_delims_encoded=" + path.pathname + " search=" + JSON.stringify(path.search) + " hash=" + JSON.stringify(path.hash));
path.pathname = "/\u00e9/\u{1F600}";
console.log("path_unicode=" + path.pathname);
const pathSetter = new URL("http://h/base/x");
pathSetter.pathname = "a/../b";
console.log("setter1=" + pathSetter.pathname);
pathSetter.pathname = "/c/./d/";
console.log("setter2=" + pathSetter.pathname);
pathSetter.pathname = "";
console.log("setter3=" + JSON.stringify(pathSetter.pathname) + " href=" + pathSetter.href);

// --- Setter: search and hash -----------------------------------------------
const q = fresh();
q.search = "x=1&y=2";
console.log("search_no_mark=" + q.search);
q.search = "?z=3";
console.log("search_with_mark=" + q.search);
q.search = "";
console.log("search_empty=" + JSON.stringify(q.search) + " href=" + q.href);
q.search = "?";
console.log("search_bare_mark=" + JSON.stringify(q.search) + " href=" + q.href);
q.search = "a b&c=d e";
console.log("search_encoded=" + q.search);
q.hash = "frag";
console.log("hash_no_mark=" + q.hash);
q.hash = "#frag2";
console.log("hash_with_mark=" + q.hash);
q.hash = "";
console.log("hash_empty=" + JSON.stringify(q.hash) + " href=" + q.href);
q.hash = "#";
console.log("hash_bare_mark=" + JSON.stringify(q.hash) + " href=" + q.href);
q.hash = "a b\"c";
console.log("hash_encoded=" + q.hash);

// --- Setter: username and password -----------------------------------------
const cred = fresh();
cred.username = "new user";
cred.password = "p:ss@word";
console.log("cred=" + cred.username + " | " + cred.password);
console.log("cred_href=" + cred.href);
cred.username = "";
cred.password = "";
console.log("cred_cleared=" + cred.href);

// --- Setter: href replaces everything; a bad value throws -------------------
const href = fresh();
href.href = "http://other.example/z?w=1";
console.log("href_set=" + href.href + " host=" + href.host + " search=" + href.search);
try {
  href.href = "not a url";
  console.log("href_bad=no-throw:" + href.href);
} catch (e: any) {
  console.log("href_bad=" + e.name + " still=" + href.href);
}

// --- Read-only accessors: origin and searchParams have no setter ------------
const originDesc = Object.getOwnPropertyDescriptor(URL.prototype, "origin") as any;
const paramsDesc = Object.getOwnPropertyDescriptor(URL.prototype, "searchParams") as any;
console.log("origin_getter=" + typeof originDesc.get + " setter=" + typeof originDesc.set);
console.log("searchParams_getter=" + typeof paramsDesc.get + " setter=" + typeof paramsDesc.set);
console.log("settable=" + ["href", "protocol", "host", "hostname", "port", "pathname", "search", "hash", "username", "password"].filter(function (k) {
  return typeof (Object.getOwnPropertyDescriptor(URL.prototype, k) as any).set === "function";
}).join(","));
