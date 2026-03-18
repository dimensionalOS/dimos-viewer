/**
 * Deno WebSocket test client for dimos-viewer event stream.
 *
 * Run with:
 *   deno run --allow-net dimos/test_ws_client.ts
 *
 * Or with a custom address:
 *   deno run --allow-net dimos/test_ws_client.ts ws://127.0.0.1:3030/ws
 */

const url = Deno.args[0] ?? "ws://127.0.0.1:3030/ws";

console.log(`Connecting to ${url} …`);

const ws = new WebSocket(url);

ws.addEventListener("open", () => {
  console.log(`[connected] Listening for events from dimos-viewer`);
});

ws.addEventListener("message", (ev) => {
  const ts = new Date().toISOString();
  try {
    const msg = JSON.parse(ev.data as string);
    if (msg.type === "heartbeat") {
      console.log(`[${ts}] heartbeat  ts=${msg.timestamp_ms}`);
    } else if (msg.type === "click") {
      console.log(
        `[${ts}] click      x=${msg.x.toFixed(3)} y=${msg.y.toFixed(3)} z=${msg.z.toFixed(3)}  entity="${msg.entity_path}"`,
      );
    } else if (msg.type === "twist") {
      console.log(
        `[${ts}] twist      lin=(${msg.linear_x.toFixed(2)}, ${msg.linear_y.toFixed(2)}, ${msg.linear_z.toFixed(2)})  ang=(${msg.angular_x.toFixed(2)}, ${msg.angular_y.toFixed(2)}, ${msg.angular_z.toFixed(2)})`,
      );
    } else if (msg.type === "stop") {
      console.log(`[${ts}] stop`);
    } else {
      console.log(`[${ts}] unknown   `, msg);
    }
  } catch {
    console.log(`[${ts}] raw:`, ev.data);
  }
});

ws.addEventListener("close", (ev) => {
  console.log(`[disconnected] code=${ev.code} reason=${ev.reason}`);
  Deno.exit(0);
});

ws.addEventListener("error", (ev) => {
  console.error(`[error]`, ev);
});
