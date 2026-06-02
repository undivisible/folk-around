import { expect, test } from "bun:test";
import worker, { type Env } from "./index";

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
