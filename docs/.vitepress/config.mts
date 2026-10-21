import { defineConfig } from 'vitepress'

// Cage 文档站：英文为默认 locale（root），中文镜像在 /zh/（docs/zh/）。
// 设计稿（docs/design.md + docs/zh/design.md）与需求整理（docs/requirements.md + docs/zh/requirements.md）
// 作为底稿保留在站外（srcExclude），站内为拆分后的栏目内容。

// ```mermaid 围栏 → <MermaidDiagram> 组件（theme/Mermaid.vue 客户端渲染）。
// 实体转义走 HTML 属性语义：静态属性、无插值，源码原样进组件。
function escapeAttr(text) {
  return text
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}

function renderMermaidFences(md) {
  const original = md.renderer.rules.fence
  md.renderer.rules.fence = (tokens, idx, options, env, self) => {
    const token = tokens[idx]
    if (token.info.trim() !== 'mermaid') {
      return original(tokens, idx, options, env, self)
    }
    const code = escapeAttr(token.content.trim())
    return `<ClientOnly><MermaidDiagram code="${code}" /></ClientOnly>\n`
  }
}

const sharedEditLink = {
  pattern: 'https://github.com/cuihairu/cage/edit/main/docs/:path',
}

const enTheme = {
  label: 'English',
  lang: 'en-US',
  link: '/',
  themeConfig: {
    nav: [
      { text: 'Home', link: '/' },
      { text: 'Architecture', link: '/architecture' },
      { text: 'Lifecycle', link: '/lifecycle' },
      { text: 'Source', link: '/source' },
      { text: 'Schema', link: '/schema' },
      { text: 'Validation', link: '/validation' },
      { text: 'Target', link: '/target' },
      { text: 'CLI', link: '/cli' },
      { text: 'Build', link: '/build' },
      { text: 'Web', link: '/web' },
      { text: 'UI', link: '/ui' },
      { text: 'UI Prototypes', link: '/design-prototypes' },
      { text: 'Assets', link: '/assets' },
      { text: 'Example', link: '/example' }
    ],

    sidebar: {
      '/': [
        {
          text: 'Overview',
          items: [
            { text: 'Architecture', link: '/architecture' },
            { text: 'Lifecycle', link: '/lifecycle' },
            { text: 'Example', link: '/example' }
          ]
        },
        {
          text: 'Data Pipeline',
          collapsed: false,
          items: [
            { text: 'Source', link: '/source' },
            { text: 'Schema', link: '/schema' },
            { text: 'Validation', link: '/validation' },
            { text: 'Target', link: '/target' }
          ]
        },
        {
          text: 'Engineering',
          collapsed: false,
          items: [
            { text: 'CLI', link: '/cli' },
            { text: 'Build', link: '/build' },
            { text: 'Assets', link: '/assets' },
            { text: 'Web', link: '/web' },
            { text: 'UI', link: '/ui' },
            { text: 'UI Prototypes', link: '/design-prototypes' }
          ]
        }
      ]
    },

    search: {
      provider: 'local',
      options: {
        translations: {
          button: { buttonText: 'Search docs', buttonAriaLabel: 'Search docs' },
          modal: {
            noResultsText: 'No results found',
            resetButtonTitle: 'Reset query',
            footer: { selectText: 'Select', navigateText: 'Navigate', closeText: 'Close' }
          }
        }
      }
    },

    outline: { level: [2, 3], label: 'On this page' },
    docFooter: { prev: 'Previous', next: 'Next' },
    lastUpdated: { text: 'Last updated' },
    darkModeSwitchLabel: 'Theme',
    lightModeSwitchTitle: 'Switch to light theme',
    darkModeSwitchTitle: 'Switch to dark theme',
    sidebarMenuLabel: 'Menu',
    returnToTopLabel: 'Back to top',
    editLink: { ...sharedEditLink, text: 'Edit this page on GitHub' },

    footer: {
      message: 'Released under the Apache-2.0 license',
      copyright: 'Copyright © 2026 cuihairu'
    }
  }
}

const zhTheme = {
  label: '简体中文',
  lang: 'zh-CN',
  link: '/zh/',
  themeConfig: {
    nav: [
      { text: '首页', link: '/zh/' },
      { text: '架构', link: '/zh/architecture' },
      { text: '生命周期', link: '/zh/lifecycle' },
      { text: 'Source', link: '/zh/source' },
      { text: 'Schema', link: '/zh/schema' },
      { text: 'Validation', link: '/zh/validation' },
      { text: 'Target', link: '/zh/target' },
      { text: 'CLI', link: '/zh/cli' },
      { text: 'Build', link: '/zh/build' },
      { text: 'Web', link: '/zh/web' },
      { text: 'UI', link: '/zh/ui' },
      { text: 'UI 原型', link: '/zh/design-prototypes' },
      { text: '资产', link: '/zh/assets' },
      { text: '完整示例', link: '/zh/example' }
    ],

    sidebar: {
      '/zh/': [
        {
          text: '概览',
          items: [
            { text: '架构', link: '/zh/architecture' },
            { text: '生命周期', link: '/zh/lifecycle' },
            { text: '完整示例', link: '/zh/example' }
          ]
        },
        {
          text: '数据管线',
          collapsed: false,
          items: [
            { text: 'Source：输入源', link: '/zh/source' },
            { text: 'Schema：结构定义', link: '/zh/schema' },
            { text: 'Validation：验证流水线', link: '/zh/validation' },
            { text: 'Target：规范化与产出', link: '/zh/target' }
          ]
        },
        {
          text: '工程化',
          collapsed: false,
          items: [
            { text: 'CLI', link: '/zh/cli' },
            { text: 'Build：确定性构建', link: '/zh/build' },
            { text: '资产：Schema 管理与加载', link: '/zh/assets' },
            { text: 'Web：Schema 编辑器', link: '/zh/web' },
            { text: 'UI：界面门面', link: '/zh/ui' },
            { text: 'UI 设计原型', link: '/zh/design-prototypes' }
          ]
        }
      ]
    },

    search: {
      provider: 'local',
      options: {
        translations: {
          button: { buttonText: '搜索文档', buttonAriaLabel: '搜索文档' },
          modal: {
            noResultsText: '无法找到相关结果',
            resetButtonTitle: '清除查询条件',
            footer: { selectText: '选择', navigateText: '切换', closeText: '关闭' }
          }
        }
      }
    },

    outline: { level: [2, 3], label: '本页目录' },
    docFooter: { prev: '上一篇', next: '下一篇' },
    lastUpdated: { text: '最后更新' },
    darkModeSwitchLabel: '主题',
    lightModeSwitchTitle: '切换到浅色',
    darkModeSwitchTitle: '切换到深色',
    sidebarMenuLabel: '菜单',
    returnToTopLabel: '回到顶部',
    editLink: { ...sharedEditLink, text: '在 GitHub 上编辑此页' },

    footer: {
      message: '基于 Apache-2.0 许可发布',
      copyright: 'Copyright © 2026 cuihairu'
    }
  }
}

export default defineConfig({
  lang: 'en-US',
  title: 'Cage',
  description: 'A configuration compiler and validator for games — compile heterogeneous configuration sources into validated, deterministically built runtime configuration assets',
  base: '/cage/',
  cleanUrls: true,
  lastUpdated: true,

  markdown: {
    config(md) {
      renderMermaidFences(md)
    },
  },

  // favicon 不走自动 base，写全路径；导航 logo 见 themeConfig.logo（自动加 base）
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/cage/favicon.svg' }]
  ],

  // 原始设计稿与需求整理作为底稿保留（中英两版），不进站点路由；审核报告
  // （含原始工具记录与 ANSI 控制字符）同样只作底稿
  srcExclude: [
    'design.md',
    'zh/design.md',
    'requirements.md',
    'zh/requirements.md',
    'review/luna-review-2026-10-18.md',
    'review/gpt6-luna-review-2026-10-18.md',
  ],

  locales: {
    root: enTheme,
    zh: zhTheme
  },

  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'Cage',

    socialLinks: [
      { icon: 'github', link: 'https://github.com/cuihairu/cage' }
    ]
  }
})
