import { expect, it } from 'vitest';
import { markToneFor } from '../../brand/markTone';

it('只有确知有事等你才是 standard；没事、还没读到、读不出都是 mono', () => {
  expect(markToneFor(1)).toBe('standard');
  expect(markToneFor(12)).toBe('standard');
  expect(markToneFor(0)).toBe('mono');
  expect(markToneFor(null)).toBe('mono');
});
