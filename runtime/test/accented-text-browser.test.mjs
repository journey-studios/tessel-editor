// BL141 in a real page (headless Chrome, SwiftShader): the runtime itself folds a decomposed
// accent into the composed letter, so a plan that skips the core's normalization still draws one
// picture; accents change the picture; a character with no glyph is the outlined box. Skipped,
// loudly, when there is no Chrome or no bundle (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { openRuntime, skipUnavailable, textLayer } from './support.mjs';

const hashOf = async (runtime, content) => {
  const done = await runtime.frame([textLayer(content)], { size: [320, 180] });
  assert.equal(done.type, 'frame.done', JSON.stringify(done.error));
  return done.hash;
};

test('accented text: composed and decomposed spellings are one picture, accents and the box differ', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello();
    const composed = await hashOf(runtime, 'Não é só açaí — ÀÉÔ');
    const decomposed = await hashOf(runtime, 'Na\u0303o e\u0301 so\u0301 ac\u0327ai\u0301 — A\u0300E\u0301O\u0302');
    assert.equal(decomposed, composed);
    assert.equal(await hashOf(runtime, 'ação'), await hashOf(runtime, 'AÇÃO'), 'lowercase draws as capitals');

    const plain = await hashOf(runtime, 'NAO E SO ACAI - AEO');
    assert.notEqual(plain, composed, 'accents change the picture');
    const tilde = await hashOf(runtime, 'AÃO');
    assert.notEqual(tilde, await hashOf(runtime, 'AÁO'), 'tilde is not acute');
    assert.notEqual(tilde, await hashOf(runtime, 'AÀO'), 'tilde is not grave');
    assert.notEqual(tilde, await hashOf(runtime, 'AAO'), 'tilde is not nothing');

    const box = await hashOf(runtime, '€€€');
    assert.notEqual(box, await hashOf(runtime, 'ÉÉÉ'), 'a covered accent is not the unknown box');
    assert.notEqual(box, await hashOf(runtime, 'EEE'));
  } finally {
    await runtime.close();
  }
});
