// Unit tests for lang-redirect.js pure helpers. Zero dependencies, run with:
//   node --test docs/.vitepress/theme/lang-redirect.test.mjs
import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  LANG_STORAGE_KEY,
  normalizePath,
  localeOf,
  isLocaleRoot,
  pageLocale,
  browserLocale,
  localeRootPath,
  decideRedirect,
  manualSwitchTarget,
} from './lang-redirect.js'

const BASE = '/cage/'

test('normalizePath strips the base and yields site-relative paths', () => {
  assert.equal(normalizePath('/cage/', BASE), '/')
  assert.equal(normalizePath('/cage', BASE), '/')
  assert.equal(normalizePath('/cage/zh/', BASE), '/zh/')
  assert.equal(normalizePath('/cage/zh', BASE), '/zh')
  assert.equal(normalizePath('/cage/schema', BASE), '/schema')
  assert.equal(normalizePath('/schema', '/'), '/schema')
})

test('localeOf recognizes only the two locale roots', () => {
  assert.equal(localeOf('/cage/', BASE), 'en')
  assert.equal(localeOf('/cage/zh/', BASE), 'zh')
  assert.equal(localeOf('/cage/zh', BASE), 'zh')
  assert.equal(localeOf('/cage/schema', BASE), null)
  assert.equal(localeOf('/cage/zh/architecture', BASE), null)
  assert.equal(localeOf('/schema', '/'), null)
})

test('isLocaleRoot gates deep pages out', () => {
  assert.equal(isLocaleRoot('/cage/', BASE), true)
  assert.equal(isLocaleRoot('/cage/zh/', BASE), true)
  assert.equal(isLocaleRoot('/cage/schema', BASE), false)
})

test('pageLocale classifies any page by its /zh/ prefix', () => {
  assert.equal(pageLocale('/cage/', BASE), 'en')
  assert.equal(pageLocale('/cage/schema', BASE), 'en')
  assert.equal(pageLocale('/cage/zh/', BASE), 'zh')
  assert.equal(pageLocale('/cage/zh/architecture', BASE), 'zh')
  assert.equal(pageLocale('/cage/zh', BASE), 'zh')
})

test('browserLocale maps zh* variants to zh, everything else to en', () => {
  assert.equal(browserLocale(['zh-CN']), 'zh')
  assert.equal(browserLocale(['zh-TW']), 'zh')
  assert.equal(browserLocale(['zh-Hans-CN']), 'zh')
  assert.equal(browserLocale(['zh']), 'zh')
  assert.equal(browserLocale(['zh-CN', 'en-US']), 'zh') // first wins
  assert.equal(browserLocale(['en-US']), 'en')
  assert.equal(browserLocale(['ja-JP']), 'en')
  assert.equal(browserLocale(['en-US', 'zh-CN']), 'en')
  assert.equal(browserLocale([]), 'en')
  assert.equal(browserLocale(undefined), 'en')
})

test('localeRootPath includes the base and the zh prefix', () => {
  assert.equal(localeRootPath('en', BASE), '/cage/')
  assert.equal(localeRootPath('zh', BASE), '/cage/zh/')
  assert.equal(localeRootPath('zh', '/'), '/zh/')
  assert.equal(localeRootPath('en', '/cage'), '/cage/')
})

test('decideRedirect follows the browser language on the opposite root', () => {
  // zh browser lands on the EN root → jump to zh root
  assert.equal(
    decideRedirect({ pathname: '/cage/', base: BASE, stored: null, languages: ['zh-CN'] }),
    '/cage/zh/'
  )
  // en browser lands on the EN root → stay
  assert.equal(
    decideRedirect({ pathname: '/cage/', base: BASE, stored: null, languages: ['en-US'] }),
    null
  )
  // zh browser already on the zh root → stay
  assert.equal(
    decideRedirect({ pathname: '/cage/zh/', base: BASE, stored: null, languages: ['zh-CN'] }),
    null
  )
  // en browser lands on the zh root → jump to EN root
  assert.equal(
    decideRedirect({ pathname: '/cage/zh/', base: BASE, stored: null, languages: ['en-US'] }),
    '/cage/'
  )
})

test('decideRedirect never redirects deep pages', () => {
  assert.equal(
    decideRedirect({ pathname: '/cage/schema', base: BASE, stored: null, languages: ['zh-CN'] }),
    null
  )
  assert.equal(
    decideRedirect({ pathname: '/cage/zh/architecture', base: BASE, stored: null, languages: ['en-US'] }),
    null
  )
})

test('decideRedirect disables auto-redirect once the user explicitly chose', () => {
  // stored 'en', zh browser, on EN root → stay (respects the explicit choice)
  assert.equal(
    decideRedirect({ pathname: '/cage/', base: BASE, stored: 'en', languages: ['zh-CN'] }),
    null
  )
  // stored 'zh', en browser, on EN root → stay (no more guessing)
  assert.equal(
    decideRedirect({ pathname: '/cage/', base: BASE, stored: 'zh', languages: ['en-US'] }),
    null
  )
  // stored 'zh', zh browser, on EN root → stay
  assert.equal(
    decideRedirect({ pathname: '/cage/', base: BASE, stored: 'zh', languages: ['zh-CN'] }),
    null
  )
})

test('manualSwitchTarget records only cross-locale root links', () => {
  // EN page → zh root = explicit switch to zh
  assert.equal(manualSwitchTarget('/cage/', '/cage/zh/', BASE), 'zh')
  // zh page → EN root = explicit switch to en
  assert.equal(manualSwitchTarget('/cage/zh/', '/cage/', BASE), 'en')
  // same-locale root (e.g. Home) is not a switch
  assert.equal(manualSwitchTarget('/cage/', '/cage/', BASE), null)
  assert.equal(manualSwitchTarget('/cage/zh/', '/cage/zh/', BASE), null)
  // deep links are never treated as an explicit locale switch
  assert.equal(manualSwitchTarget('/cage/', '/cage/zh/architecture', BASE), null)
  assert.equal(manualSwitchTarget('/cage/schema', '/cage/zh/architecture', BASE), null)
  // deep source page → cross root link (the dropdown) still records a switch
  assert.equal(manualSwitchTarget('/cage/schema', '/cage/zh/', BASE), 'zh')
  assert.equal(manualSwitchTarget('/cage/zh/architecture', '/cage/', BASE), 'en')
})

test('LANG_STORAGE_KEY is stable for the browser wiring', () => {
  assert.equal(LANG_STORAGE_KEY, 'cage-docs-lang')
})
