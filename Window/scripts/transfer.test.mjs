import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { runInNewContext } from 'node:vm';
import { build } from 'vite';

const output = await build({ configFile: false, logLevel: 'silent', build: { write: false, minify: false,
  lib: { entry: fileURLToPath(new URL('../src/bridge/transfer.ts', import.meta.url)), formats: ['cjs'] } } });
const compiled = (Array.isArray(output) ? output : [output]).flatMap((bundle) => bundle.output).find((item) => item.type === 'chunk').code;
const module = { exports: {} };
runInNewContext(compiled, { module, exports: module.exports, require: createRequire(import.meta.url), TextDecoder });
const { learningTemplate } = module.exports;
test('앱에서 받는 양식은 단답형 1.0과 서술형 2.0을 구분한다', () => {
  const short = JSON.parse(learningTemplate('1.0')); const descriptive = JSON.parse(learningTemplate('2.0'));
  assert.equal(short.schema_version, '1.0'); assert.equal(short.items[0].type, undefined);
  assert.equal(descriptive.schema_version, '2.0'); assert.equal(descriptive.items[0].type, 'descriptive');
  assert.ok(descriptive.items[0].reference_answer); assert.equal(descriptive.items[0].key_points.length, 1);
});
