// Unit tests for the P1-1 three-tier mode contract (modes.js). Zero
// dependencies, run with:
//   node --test docs/public/editor/modes.test.mjs
import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  MODES,
  MODE_STORAGE_KEY,
  isMode,
  applyExpertText,
  ruleFormat,
  addRule,
} from './modes.js'

const DOC = {
  tables: {
    Monster: {
      name: 'Monster',
      primary_key: ['id'],
      fields: {
        id: { name: 'id', type: { kind: 'Int32' }, required: true },
        best_item: { name: 'best_item', type: { kind: 'Int32' } },
      },
    },
  },
  enums: {},
}

test('MODES is exactly the three tiers and isMode gates them', () => {
  assert.deepEqual(MODES, ['basic', 'rules', 'expert'])
  assert.equal(isMode('basic'), true)
  assert.equal(isMode('rules'), true)
  assert.equal(isMode('expert'), true)
  assert.equal(isMode('yaml'), false)
  assert.equal(isMode(undefined), false)
})

test('MODE_STORAGE_KEY is stable for the browser wiring', () => {
  assert.equal(MODE_STORAGE_KEY, 'cage-editor-mode')
})

test('applyExpertText round-trips the semantic model byte-for-byte', () => {
  const text = JSON.stringify(DOC, null, 2)
  const r = applyExpertText(text)
  assert.equal(r.error, undefined)
  assert.deepEqual(r.doc, DOC)
  // The expert text of the applied doc is the same text: the expert tier
  // edits the one model, it does not fork a parallel representation.
  assert.equal(JSON.stringify(r.doc, null, 2), text)
})

test('applyExpertText rejects broken JSON atomically (no doc)', () => {
  const r = applyExpertText('{"tables": {')
  assert.ok(r.error.includes('JSON 无法解析'), r.error)
  assert.equal(r.doc, undefined)
})

test('applyExpertText rejects non-object documents', () => {
  for (const text of ['[]', '"doc"', '42', 'null']) {
    const r = applyExpertText(text)
    assert.ok(r.error.includes('JSON 对象'), `${text}: ${r.error}`)
    assert.equal(r.doc, undefined)
  }
})

test('addRule appends to the ONE rules representation (禁三套规则格式)', () => {
  const doc = structuredClone(DOC)
  const r = addRule(doc, 'Monster', 'best_item')
  assert.equal(r.ok, true, r.error)

  // The edit landed in the same array every tier reads.
  const rules = ruleFormat(doc, 'Monster', 'best_item')
  assert.equal(rules, doc.tables.Monster.fields.best_item.rules)
  assert.deepEqual(rules, [{ name: 'rule_1', assert: '' }])

  // The shape is the Rust ExpressionRule: name/assert required, message /
  // warning_only optional — no third format side channel anywhere.
  assert.deepEqual(Object.keys(rules[0]).sort(), ['assert', 'name'])
})

test('addRule keeps existing rules and names sequentially', () => {
  const doc = structuredClone(DOC)
  doc.tables.Monster.fields.best_item.rules = [
    { name: 'cap', assert: 'best_item <= 200', message: 'too pricey', warning_only: true },
  ]
  assert.equal(addRule(doc, 'Monster', 'best_item').ok, true)
  const rules = ruleFormat(doc, 'Monster', 'best_item')
  assert.equal(rules.length, 2)
  assert.deepEqual(rules[0], {
    name: 'cap', assert: 'best_item <= 200', message: 'too pricey', warning_only: true,
  })
  assert.deepEqual(rules[1], { name: 'rule_2', assert: '' })
})

test('the expert tier round-trips rules intact (one model, three tiers)', () => {
  const doc = structuredClone(DOC)
  addRule(doc, 'Monster', 'best_item')
  const r = applyExpertText(JSON.stringify(doc, null, 2))
  assert.equal(r.error, undefined)
  assert.deepEqual(ruleFormat(r.doc, 'Monster', 'best_item'), [
    { name: 'rule_1', assert: '' },
  ])
})

test('addRule refuses unknown fields', () => {
  assert.equal(addRule(structuredClone(DOC), 'Monster', 'ghost').ok, false)
  assert.equal(addRule(structuredClone(DOC), 'Ghost', 'id').ok, false)
  assert.equal(addRule(null, 'Monster', 'id').ok, false)
})

test('ruleFormat reads the single representation and nothing else', () => {
  assert.deepEqual(ruleFormat(DOC, 'Monster', 'id'), undefined)
  assert.equal(ruleFormat(DOC, 'Ghost', 'id'), undefined)
  assert.equal(ruleFormat(null, 'Monster', 'id'), undefined)
})
