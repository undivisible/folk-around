import { expect, test } from "bun:test";
import worker, { SignalRoom, type Env } from "./index";

test("health endpoint returns service status", async () => {
  const response = await worker.fetch(
    new Request("https://folk-around.test/health"),
    {} as Env,
  );

  expect(response.status).toBe(200);
  await expect(response.json()).resolves.toEqual({
    ok: true,
    service: "folk-around-signal",
  });
});

test("root endpoint returns service page without asset binding", async () => {
  const response = await worker.fetch(
    new Request("https://folk-around.test/"),
    {} as Env,
  );

  expect(response.status).toBe(200);
  expect(response.headers.get("content-type")).toContain("text/html");
  await expect(response.text()).resolves.toContain("Folk Around Signalling");
});

class FakeSocket {
  closed = false;
  sent: string[] = [];

  send(message: string) {
    this.sent.push(message);
  }

  close(code?: number, reason?: string) {
    this.closed = true;
  }
}

function fakeJoin(room: SignalRoom, identity: string): FakeSocket {
  const socket = new FakeSocket();
  (room as any).handleMessage(socket, { type: "join", identity });
  return socket;
}

test("signal room rejects join without hex identity", () => {
  const room = new SignalRoom();
  const socket = new FakeSocket();

  (room as any).handleMessage(socket, { type: "join", identity: "bad" });

  expect(socket.sent).toEqual([
    JSON.stringify({ type: "error", message: "Invalid identity" }),
  ]);
});

test("signal room forwards relay only to target peer", () => {
  const room = new SignalRoom();
  const firstIdentity =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const secondIdentity =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

  const first = fakeJoin(room, firstIdentity);
  const second = fakeJoin(room, secondIdentity);
  first.sent = [];
  second.sent = [];

  (room as any).handleMessage(first, {
    type: "relay",
    from: firstIdentity,
    to: secondIdentity,
    data: { jsonrpc: "2.0", id: 1, method: "ping" },
  });

  expect(second.sent).toEqual([
    JSON.stringify({
      type: "relay",
      from: firstIdentity,
      data: { jsonrpc: "2.0", id: 1, method: "ping" },
    }),
  ]);
  expect(first.sent).toEqual([]);
});

test("signal room forwards opaque encrypted relay data", () => {
  const room = new SignalRoom();
  const firstIdentity =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const secondIdentity =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
  const encryptedData =
    "v1:XS6CFkLB4xd4RZ+7gVsDCMf/7oOfDK83SwcKz00y60U=:PZ0/zeyJub25jZSI6InRlc3QiLCJjdCI6Ilp6In0=";

  const first = fakeJoin(room, firstIdentity);
  const second = fakeJoin(room, secondIdentity);
  first.sent = [];
  second.sent = [];

  (room as any).handleMessage(first, {
    type: "relay",
    from: firstIdentity,
    to: secondIdentity,
    data: encryptedData,
  });

  expect(second.sent).toEqual([
    JSON.stringify({
      type: "relay",
      from: firstIdentity,
      data: encryptedData,
    }),
  ]);
  expect(first.sent).toEqual([]);
});

test("signal room rejects spoofed sender identity", () => {
  const room = new SignalRoom();
  const firstIdentity =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const secondIdentity =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

  const first = fakeJoin(room, firstIdentity);
  fakeJoin(room, secondIdentity);
  first.sent = [];

  (room as any).handleMessage(first, {
    type: "relay",
    from: secondIdentity,
    to: secondIdentity,
    data: { jsonrpc: "2.0", id: 1, method: "ping" },
  });

  expect(first.sent).toEqual([
    JSON.stringify({ type: "error", message: "Sender identity mismatch" }),
  ]);
});

test("signal room rejects relay with invalid target identity", () => {
  const room = new SignalRoom();
  const firstIdentity =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

  const first = fakeJoin(room, firstIdentity);
  first.sent = [];

  (room as any).handleMessage(first, {
    type: "relay",
    from: firstIdentity,
    to: "not-a-peer",
    data: "encrypted",
  });

  expect(first.sent).toEqual([
    JSON.stringify({ type: "error", message: "Invalid peer identity" }),
  ]);
});

test("signal room rejects oversized relay data", () => {
  const room = new SignalRoom();
  const firstIdentity =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const secondIdentity =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

  const first = fakeJoin(room, firstIdentity);
  const second = fakeJoin(room, secondIdentity);
  first.sent = [];
  second.sent = [];

  (room as any).handleMessage(first, {
    type: "relay",
    from: firstIdentity,
    to: secondIdentity,
    data: "x".repeat(256 * 1024),
  });

  expect(first.sent).toEqual([
    JSON.stringify({ type: "error", message: "Message too large" }),
  ]);
  expect(second.sent).toEqual([]);
});

test("signal room rejects join during grace period", () => {
  const room = new SignalRoom();
  const identity =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

  const first = fakeJoin(room, identity);
  expect(first.closed).toBe(false);

  // second join should be rejected (within grace period)
  const second = new FakeSocket();
  (room as any).handleMessage(second, { type: "join", identity });

  expect(second.closed).toBe(true);
  expect(second.sent).toEqual([
    JSON.stringify({
      type: "error",
      message: "Identity recently joined, try later",
    }),
  ]);
});
