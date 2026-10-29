// Cross-runtime: TextEncoderStream, TextDecoderStream, Blob.stream(),
// and CompressionStream/DecompressionStream round-trip.
// Folded from streams/101_textdecoder_stream, streams/86_blob_stream and
// streams/88_compression_stream (compressed byte length omitted: zlib output
// differs between runtimes, so only the magic bytes and round-trip are checked).

async function readChunks(stream: ReadableStream<any>): Promise<any[]> {
  const reader = stream.getReader();
  const out: any[] = [];
  while (true) {
    const part = await reader.read();
    if (part.done) break;
    out.push(part.value);
  }
  return out;
}

async function main() {
  // 1. TextEncoderStream -> TextDecoderStream round-trip, with a non-ASCII code point.
  const encoder = new TextEncoderStream();
  const decoder = new TextDecoderStream();
  const writer = encoder.writable.getWriter();
  const reader = encoder.readable.pipeThrough(decoder).getReader();
  const writing = (async () => {
    await writer.write("caf");
    await writer.write("\u00e9");
    await writer.close();
  })();
  const decoded: string[] = [];
  while (true) {
    const part = await reader.read();
    if (part.done) break;
    decoded.push(String(part.value));
  }
  await writing;
  const joined = decoded.join("");
  const codes: number[] = [];
  for (let i = 0; i < joined.length; i++) {
    codes.push(joined.charCodeAt(i));
  }
  console.log("text.chunks=" + decoded.length);
  console.log("text.codes=" + codes.join(","));
  console.log("text.length=" + joined.length);

  // 2. TextEncoderStream emits Uint8Array chunks; check their bytes.
  const encOnly = new TextEncoderStream();
  const encWriter = encOnly.writable.getWriter();
  encWriter.write("AB");
  encWriter.close();
  const encChunks = await readChunks(encOnly.readable);
  const firstChunk = encChunks[0];
  console.log("enc.chunks=" + encChunks.length);
  console.log("enc.isUint8=" + (firstChunk instanceof Uint8Array));
  console.log("enc.bytes=" + Array.from(firstChunk as Uint8Array).join(","));

  // 3. Blob.stream() yields the blob's bytes as Uint8Array chunks in order.
  const blob = new Blob(["ab"]);
  const blobChunks = await readChunks(blob.stream());
  const blobBytes: number[] = [];
  for (const chunk of blobChunks) {
    for (const byte of chunk as Uint8Array) {
      blobBytes.push(byte);
    }
  }
  console.log("blob.bytes=" + blobBytes.join(","));
  console.log("blob.size=" + blob.size);

  // 4. Blob.stream() over a multi-part blob keeps the byte order.
  const multi = new Blob(["hel", "lo ", "world"]);
  const multiText = new TextDecoder().decode(
    (await readChunks(multi.stream())).reduce(
      (acc: Uint8Array, chunk: Uint8Array) => {
        const merged = new Uint8Array(acc.length + chunk.length);
        merged.set(acc, 0);
        merged.set(chunk, acc.length);
        return merged;
      },
      new Uint8Array(0),
    ),
  );
  console.log("blob.multi=" + multiText);

  // 5. gzip round-trip: magic bytes, then decompress back to the original text.
  const payload = new TextEncoder().encode("hello hello");
  const compressor = new CompressionStream("gzip");
  const compWriter = compressor.writable.getWriter();
  const compWriting = (async () => {
    await compWriter.write(payload);
    await compWriter.close();
  })();
  const compressed = await readChunks(compressor.readable);
  await compWriting;
  let compressedBytes: number[] = [];
  for (const chunk of compressed) {
    for (const byte of chunk as Uint8Array) {
      compressedBytes.push(byte);
    }
  }
  console.log("gzip.magic=" + compressedBytes[0] + "," + compressedBytes[1]);

  const decompressor = new DecompressionStream("gzip");
  const decompWriter = decompressor.writable.getWriter();
  const decompWriting = (async () => {
    await decompWriter.write(new Uint8Array(compressedBytes));
    await decompWriter.close();
  })();
  const restored = await readChunks(decompressor.readable);
  await decompWriting;
  let restoredBytes: number[] = [];
  for (const chunk of restored) {
    for (const byte of chunk as Uint8Array) {
      restoredBytes.push(byte);
    }
  }
  console.log("gzip.restored=" + new TextDecoder().decode(new Uint8Array(restoredBytes)));
  console.log("gzip.restoredLength=" + restoredBytes.length);
}

main();
