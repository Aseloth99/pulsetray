import test from "node:test";
import assert from "node:assert/strict";
import { lib, read } from "./load.mjs";

const { mean } = await lib("format");

test("mean averages the readings and skips missing ones", () => {
  assert.equal(mean([10, 20, 30]), 20);
  assert.equal(mean([10, null, 30, null]), 20, "gaps are not zeros");
  assert.equal(mean([0, 0, 100]), 100 / 3);
  assert.equal(mean([5]), 5);
});

test("mean of nothing is null, not 0 or NaN", () => {
  assert.equal(mean([]), null);
  assert.equal(mean([null, null]), null);
  assert.equal(mean([Number.NaN, Infinity]), null);
});

test("the chart header shows the average before the peak, in both languages", () => {
  const spark = read("src/lib/Spark.svelte");
  assert.match(spark, /tr\("avgShort"\)[\s\S]*tr\("peak"\)/);
  const i18n = read("src/lib/i18n.ts");
  assert.match(i18n, /avgShort: "avg"/);
  assert.match(i18n, /avgShort: "ort"/);
});
