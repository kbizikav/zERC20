import { describe, expect, test } from 'vitest';
import { GLOBAL_TRANSFER_TREE_HEIGHT } from '../../constants.js';
import * as proofUtils from '../proofUtils.js';

describe('appendDummySteps', () => {
  test('allocates dummy leaves beyond the reachable tree range', () => {
    const steps: any[] = [];

    proofUtils.appendDummySteps(steps);

    expect(steps.length).toBeGreaterThan(0);
    const maxLeaves = 1n << BigInt(GLOBAL_TRANSFER_TREE_HEIGHT);
    const dummyCount = BigInt(steps.length);
    const expectedFirstLeaf = (maxLeaves - 1n - dummyCount).toString();
    expect(steps[0].leafIndex).toBe(expectedFirstLeaf);
    const expectedLastLeaf = (maxLeaves - 2n).toString();
    expect(steps.at(-1)?.leafIndex).toBe(expectedLastLeaf);
    expect(steps[0].siblings).toHaveLength(GLOBAL_TRANSFER_TREE_HEIGHT);
    expect(steps[0].is_dummy).toBe(true);
  });
});
