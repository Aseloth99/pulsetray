// Loads a TypeScript module of src/lib for a test (Node strips the types itself; see ts-resolve.mjs for the imports).
import { register } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

register("./ts-resolve.mjs", import.meta.url);

export const lib = (name) => import(new URL(`../src/lib/${name}.ts`, import.meta.url).href);
export const read = (rel) => readFileSync(fileURLToPath(new URL(`../${rel}`, import.meta.url)), "utf8");
