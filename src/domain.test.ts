import { describe, it, expect } from 'vitest';
import { remaining, freshness, windowState, countdown } from './domain';
import type { Snapshot } from './types';
const snapshot: Snapshot = { provider:'codex', state:'ready', accountId:'a',accountLabel:null,observedAt:1000, windows:[],source:'test',message:null };
describe('usage semantics', () => {
 it('keeps absent values separate from exhausted allowance',()=>{expect(remaining(null)).toBeNull();expect(remaining(NaN)).toBeNull();expect(remaining(100)).toBe(0);expect(remaining(25)).toBe(75);expect(remaining(-3)).toBe(100)});
 it('expires at exactly ten minutes and does not accept future observations',()=>{expect(freshness(snapshot,1599)).toBe('fresh');expect(freshness(snapshot,1600)).toBe('stale');expect(freshness(snapshot,900)).toBe('stale')});
 it('uses Claude Desktop probe cadence for cache freshness',()=>{const cached={...snapshot,source:'Claude Desktop usage cache'};expect(freshness(cached,2199)).toBe('fresh');expect(freshness(cached,2200)).toBe('stale')});
 it('never invents a reset',()=>{expect(windowState(snapshot,{id:'a',label:'x',resetsAt:1200,usedPercent:99,durationMins:300},1200)).toBe('awaiting');expect(countdown(null,1000,20)).toBe('Reset time unavailable');expect(countdown(null,1000,0)).toBe('Full allowance available');expect(countdown(1200,1200)).toBe('Awaiting update')});
});
