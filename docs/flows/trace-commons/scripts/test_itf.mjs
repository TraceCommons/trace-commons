import assert from 'node:assert/strict';
import { decode } from './itf.mjs';

// Variant-valued map keys must not collapse through JS's '[object Object]'.
const variant = tag => ({tag, value: {'#tup': []}});
const result = decode({'#map': [[variant('Personal'), 1], [variant('Work'), 2]]});
assert.equal(Object.keys(result).length, 2, 'distinct variant keys must survive decoding');
assert.equal(result[JSON.stringify({Personal: {tag: 'UNIT'}})], 1);
assert.equal(result[JSON.stringify({Work: {tag: 'UNIT'}})], 2);
// Use the same one-based sequence projection as the pinned TLC view frontend.
assert.deepEqual(decode({'#tup': ['first', 'second']}), {'1': 'first', '2': 'second'});
assert.deepEqual(decode({'#set': [{'#bigint': '2'}, false]}), [2, false]);
assert.throws(() => decode({'#bigint': '9007199254740993'}), /safe integer/);
console.log('ITF decoding tests passed (variant keys, sequences, sets, integer bounds).');
