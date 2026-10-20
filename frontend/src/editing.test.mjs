import assert from 'node:assert/strict'
import { removedDuration, readCuts } from './editing.ts'

const cuts = [
  { id: 'a', media_id: 'one', start_ms: 1000, end_ms: 3000 },
  { id: 'b', media_id: 'one', start_ms: 2000, end_ms: 4000 },
  { id: 'c', media_id: 'two', start_ms: 0, end_ms: 8000 },
]
assert.equal(removedDuration(cuts, 'one', 5000), 3000)
assert.equal(removedDuration([...cuts].reverse(), 'one', 3500), 2500)
assert.equal(removedDuration(cuts, 'missing', 5000), 0)
assert.deepEqual(readCuts(JSON.stringify(cuts)), cuts)
assert.deepEqual(readCuts('{broken'), [])
assert.deepEqual(readCuts('{}'), [])
assert.deepEqual(readCuts('[null,{"id":"x","media_id":"m","start_ms":-1,"end_ms":4}]'), [])
console.log('Revisão: sobreposição, limites, isolamento entre mídias e rascunhos inválidos verificados.')
