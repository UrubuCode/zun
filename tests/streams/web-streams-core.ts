// Cross-runtime: WHATWG ReadableStream, WritableStream, TransformStream.
// Folded from streams/64_readable_stream and streams/96_streams_transform.

async function readAll(stream: ReadableStream<any>): Promise<string[]> {
  const reader = stream.getReader();
  const out: string[] = [];
  while (true) {
    const part = await reader.read();
    if (part.done) break;
    out.push(String(part.value));
  }
  return out;
}

async function main() {
  // 1. Basic pull from a stream closed in start().
  const basic = new ReadableStream({
    start(controller) {
      controller.enqueue("a");
      controller.enqueue("b");
      controller.close();
    },
  });
  const basicOut = await readAll(basic);
  console.log("basic.out=" + basicOut.join(","));
  console.log("basic.count=" + basicOut.length);

  // 2. Reader done state is sticky after close.
  const doneReader = new ReadableStream({
    start(controller) {
      controller.enqueue(1);
      controller.close();
    },
  }).getReader();
  const first = await doneReader.read();
  const second = await doneReader.read();
  const third = await doneReader.read();
  console.log("done.first=" + first.value + "/" + first.done);
  console.log("done.second=" + second.value + "/" + second.done);
  console.log("done.third=" + third.value + "/" + third.done);

  // 3. Pull-based source: pull() is driven by reads, values come out in order.
  let counter = 0;
  const pulled = new ReadableStream({
    pull(controller) {
      counter += 1;
      if (counter > 3) {
        controller.close();
      } else {
        controller.enqueue("p" + counter);
      }
    },
  });
  const pulledOut = await readAll(pulled);
  console.log("pull.out=" + pulledOut.join(","));

  // 4. Error propagation: controller.error() rejects the pending read.
  let failPulls = 0;
  const failing = new ReadableStream({
    pull(controller) {
      failPulls += 1;
      if (failPulls === 1) {
        controller.enqueue("ok");
      } else {
        controller.error(new Error("boom"));
      }
    },
  });
  const failReader = failing.getReader();
  const okChunk = await failReader.read();
  console.log("error.first=" + okChunk.value);
  try {
    await failReader.read();
    console.log("error.second=none");
  } catch (e) {
    console.log("error.second=" + (e as Error).message);
  }

  // 5. ReadableStream.from over an array (when available).
  if (typeof (ReadableStream as any).from === "function") {
    const fromOut = await readAll((ReadableStream as any).from(["x", "y", "z"]));
    console.log("from.out=" + fromOut.join(","));
  } else {
    console.log("from.out=unavailable");
  }

  // 6. WritableStream sink receives writes in order, close() resolves after sink close.
  const sunk: string[] = [];
  let sinkClosed = false;
  const sink = new WritableStream({
    write(chunk) {
      sunk.push(String(chunk));
    },
    close() {
      sinkClosed = true;
    },
  });
  const sinkWriter = sink.getWriter();
  await sinkWriter.write("w1");
  await sinkWriter.write("w2");
  await sinkWriter.write("w3");
  await sinkWriter.close();
  console.log("sink.out=" + sunk.join(","));
  console.log("sink.closed=" + sinkClosed);

  // 7. TransformStream pipeline: uppercase transform, writer and reader run concurrently.
  const upper = new TransformStream({
    transform(chunk, controller) {
      controller.enqueue(String(chunk).toUpperCase());
    },
  });
  const upperWriter = upper.writable.getWriter();
  const upperReader = upper.readable.getReader();
  const writing = (async () => {
    await upperWriter.write("ab");
    await upperWriter.write("cd");
    await upperWriter.close();
  })();
  const upperOut: string[] = [];
  while (true) {
    const part = await upperReader.read();
    if (part.done) break;
    upperOut.push(String(part.value));
  }
  await writing;
  console.log("transform.out=" + upperOut.join(","));

  // 8. TransformStream with multiple enqueues per chunk and a flush() tail.
  const splitter = new TransformStream({
    transform(chunk, controller) {
      const text = String(chunk);
      for (const ch of text) {
        controller.enqueue(ch);
      }
    },
    flush(controller) {
      controller.enqueue("<end>");
    },
  });
  const splitWriter = splitter.writable.getWriter();
  const splitReader = splitter.readable.getReader();
  const splitWriting = (async () => {
    await splitWriter.write("xy");
    await splitWriter.write("z");
    await splitWriter.close();
  })();
  const splitOut: string[] = [];
  while (true) {
    const part = await splitReader.read();
    if (part.done) break;
    splitOut.push(String(part.value));
  }
  await splitWriting;
  console.log("split.out=" + splitOut.join(","));

  // 9. pipeTo from a ReadableStream into a WritableStream, then pipeThrough a TransformStream.
  const piped: string[] = [];
  await new ReadableStream({
    start(controller) {
      controller.enqueue("p-1");
      controller.enqueue("p-2");
      controller.close();
    },
  }).pipeTo(
    new WritableStream({
      write(chunk) {
        piped.push(String(chunk));
      },
    }),
  );
  console.log("pipeTo.out=" + piped.join(","));

  const chained = await readAll(
    new ReadableStream({
      start(controller) {
        controller.enqueue("c1");
        controller.enqueue("c2");
        controller.close();
      },
    }).pipeThrough(
      new TransformStream({
        transform(chunk, controller) {
          controller.enqueue(String(chunk) + "!");
        },
      }),
    ),
  );
  console.log("pipeThrough.out=" + chained.join(","));
}

main();
