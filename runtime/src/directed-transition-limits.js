// The hostile-input ceilings of a plan with a transition (F064, P4b S2-R0): items, depth and bytes.
//
// The numbers are the S0 contract's, the same as the core's (`MAX_TRANSITION_PLAN_ITEMS`,
// `MAX_TRANSITION_PLAN_BYTES`, crates/core/src/scene_plan/transition.rs). They are checked BEFORE
// any element of a plan is read, so a plan that is too big costs a few array lengths and not a
// walk, and a chain of neighbours never recurses: depth is 1 and an `other` that has a
// `transition` of its own is refused without being entered.
//
// The checks are pure (no GL, no DOM, no clock) and never mutate what they read.

import { invalid, isObject } from './directed-deck.js';

/** The most items (nodes, layers, splats, passes, resources and lights) of the two decks together. */
export const MAX_TRANSITION_PLAN_ITEMS = 20_000;

/** The most bytes of the JSON TEXT of a plan with a transition (UTF-8, the two decks together). */
export const MAX_TRANSITION_PLAN_BYTES = 4_194_304;

/** The arrays of a deck whose lengths make its items. */
const ITEM_ARRAYS = ['nodes', 'layers', 'splats', 'fx', 'resources', 'lights'];

/**
 * The items one deck counts: the lengths of its own arrays. An absent key, a key that is not an array
 * and a deck that is not an object count 0 (reading it later is refused with its own pointer).
 */
export const deckItems = (deck) => {
  if (!isObject(deck)) return 0;
  let items = 0;
  for (const key of ITEM_ARRAYS) {
    if (Object.hasOwn(deck, key) && Array.isArray(deck[key])) items += deck[key].length;
  }
  return items;
};

/** The item ceiling over both decks, from the array lengths alone. `transition` is the plan's own. */
export const checkItemCeiling = (plan, transition) => {
  const other = isObject(transition) && Object.hasOwn(transition, 'other') ? transition.other : undefined;
  const items = deckItems(plan) + deckItems(other);
  if (items > MAX_TRANSITION_PLAN_ITEMS) {
    throw invalid(
      '/transition',
      `the two decks hold ${items} items, more than the ${MAX_TRANSITION_PLAN_ITEMS} a plan with a transition may have`,
    );
  }
};

/** Depth 1: the neighbour has no `transition` of its own. Detected before it is read, nothing recurses. */
export const checkDepth = (transition) => {
  if (isObject(transition.other) && Object.hasOwn(transition.other, 'transition')) {
    throw invalid('/transition/other/transition', 'a transition does not nest: the neighbour deck has none (depth 1)');
  }
};

/**
 * The length of `text` in UTF-8 bytes, counted per UTF-16 unit without building the buffer: 1, 2 or 3
 * bytes for a unit and 4 for a surrogate pair; a lone surrogate counts 3 (what `TextEncoder` writes for
 * U+FFFD). Stops as soon as the count passes `limit`, so a huge text is not walked to its end.
 */
export const utf8Length = (text, limit = Number.POSITIVE_INFINITY) => {
  let bytes = 0;
  for (let index = 0; index < text.length; index += 1) {
    const unit = text.charCodeAt(index);
    if (unit < 0x80) {
      bytes += 1;
    } else if (unit < 0x800) {
      bytes += 2;
    } else if (unit >= 0xd800 && unit <= 0xdbff && index + 1 < text.length) {
      const next = text.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        index += 1;
      } else {
        bytes += 3;
      }
    } else {
      bytes += 3;
    }
    if (bytes > limit) return bytes;
  }
  return bytes;
};

/** The byte ceiling of the TEXT the host sent, measured on that text and before it is parsed. */
export const checkTextBytes = (text) => {
  const bytes = utf8Length(text, MAX_TRANSITION_PLAN_BYTES);
  if (bytes > MAX_TRANSITION_PLAN_BYTES) {
    throw invalid(
      '/transition',
      `the plan is more than ${MAX_TRANSITION_PLAN_BYTES} bytes, the most a plan with a transition may have`,
    );
  }
};
