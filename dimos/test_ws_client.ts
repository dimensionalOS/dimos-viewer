/**
 * Deno WebSocket test server for dimos-viewer event stream.
 *
 * Listens for incoming WebSocket connections from the viewer and logs
 * all received events (click, twist, stop).
 *
 * Run with:
 *   deno run --allow-net dimos/test_ws_client.ts
 *
 * Or with a custom port:
 *   deno run --allow-net dimos/test_ws_client.ts 3030
 */

const port = parseInt(Deno.args[0] ?? "3030", 10);

Deno.serve({ port }, (req) => {
  if (req.headers.get("upgrade") === "websocket") {
    const { socket, response } = Deno.upgradeWebSocket(req);

    socket.addEventListener("open", () => {
      console.log("[connected] dimos-viewer client connected");
    });

    socket.addEventListener("message", (ev) => {
      const ts = new Date().toISOString();
      try {
        const msg = JSON.parse(ev.data as string);
        if (msg.type === "click") {
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

    socket.addEventListener("close", (ev) => {
      console.log(`[disconnected] code=${ev.code} reason=${ev.reason}`);
    });

    socket.addEventListener("error", (ev) => {
      console.error(`[error]`, ev);
    });

    return response;
  }
  return new Response("Not a websocket request", { status: 400 });
});

console.log(`WebSocket test server listening on ws://localhost:${port}/ws`);
console.log(`Start the viewer with: dimos-viewer --ws-url ws://localhost:${port}/ws`);
