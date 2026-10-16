import { defineConfig } from 'vitepress'

// Cage 文档站：设计稿（docs/design.md）+ 需求整理（docs/需求整理.md）拆分为七个栏目页。
// 原始底稿保留在 docs/ 下不入站（srcExclude），站内为拆分后的栏目内容。

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

const zhTheme = {
  label: '简体中文',
  lang: 'zh-CN',
  link: '/',
  themeConfig: {
    nav: [
      { text: '首页', link: '/' },
      { text: '架构', link: '/architecture' },
      { text: '生命周期', link: '/lifecycle' },
      { text: 'Source', link: '/source' },
      { text: 'Schema', link: '/schema' },
      { text: 'Validation', link: '/validation' },
      { text: 'Target', link: '/target' },
      { text: 'CLI', link: '/cli' },
      { text: 'Build', link: '/build' },
      { text: 'Web', link: '/web' },
      { text: 'UI', link: '/ui' },
      { text: 'UI 原型', link: '/design-prototypes' },
      { text: '资产', link: '/assets' },
      { text: '完整示例', link: '/example' }
    ],

    sidebar: {
      '/': [
        {
          text: '概览',
          items: [
            { text: '架构', link: '/architecture' },
            { text: '生命周期', link: '/lifecycle' },
            { text: '完整示例', link: '/example' }
          ]
        },
        {
          text: '数据管线',
          collapsed: false,
          items: [
            { text: 'Source：输入源', link: '/source' },
            { text: 'Schema：结构定义', link: '/schema' },
            { text: 'Validation：验证流水线', link: '/validation' },
            { text: 'Target：规范化与产出', link: '/target' }
          ]
        },
        {
          text: '工程化',
          collapsed: false,
          items: [
            { text: 'CLI', link: '/cli' },
            { text: 'Build：确定性构建', link: '/build' },
            { text: '资产：Schema 管理与加载', link: '/assets' },
            { text: 'Web：Schema 编辑器', link: '/web' },
            { text: 'UI：界面门面', link: '/ui' },
            { text: 'UI 设计原型', link: '/design-prototypes' }
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
    editLink: { ...sharedEditLink, text: '在 GitHub 上编辑此页' }
  }
}

const enTheme = {
  label: 'English',
  lang: 'en-US',
  link: '/en/',
  themeConfig: {
    nav: [
      { text: 'Home', link: '/en/' },
      { text: 'Architecture', link: '/en/architecture' },
      { text: 'Lifecycle', link: '/en/lifecycle' },
      { text: 'Source', link: '/en/source' },
      { text: 'Schema', link: '/en/schema' },
      { text: 'Validation', link: '/en/validation' },
      { text: 'Target', link: '/en/target' },
      { text: 'CLI', link: '/en/cli' },
      { text: 'Build', link: '/en/build' },
      { text: 'Web', link: '/en/web' },
      { text: 'UI', link: '/en/ui' },
      { text: 'UI Prototypes', link: '/en/design-prototypes' },
      { text: 'Assets', link: '/en/assets' },
      { text: 'Example', link: '/en/example' }
    ],

    sidebar: {
      '/en/': [
        {
          text: 'Overview',
          items: [
            { text: 'Architecture', link: '/en/architecture' },
            { text: 'Lifecycle', link: '/en/lifecycle' },
            { text: 'Example', link: '/en/example' }
          ]
        },
        {
          text: 'Data Pipeline',
          collapsed: false,
          items: [
            { text: 'Source', link: '/en/source' },
            { text: 'Schema', link: '/en/schema' },
            { text: 'Validation', link: '/en/validation' },
            { text: 'Target', link: '/en/target' }
          ]
        },
        {
          text: 'Engineering',
          collapsed: false,
          items: [
            { text: 'CLI', link: '/en/cli' },
            { text: 'Build', link: '/en/build' },
            { text: 'Assets', link: '/en/assets' },
            { text: 'Web', link: '/en/web' },
            { text: 'UI', link: '/en/ui' },
            { text: 'UI Prototypes', link: '/en/design-prototypes' }
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
    editLink: { ...sharedEditLink, text: 'Edit this page on GitHub' }
  }
}

export default defineConfig({
  lang: 'zh-CN',
  title: 'Cage',
  description: '游戏配置编译与验证框架——把异构配置源编译为经过验证、确定性构建的运行时配置资产',
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

  // 原始设计稿与需求整理作为底稿保留，不进站点路由
  srcExclude: ['design.md', '需求整理.md', 'requirements.md'],

  // 英文源文件保留 <name>.en.md，站点路由归入 /en/ 以启用 locale 主题。
  rewrites: {
    'index.en.md': 'en/index.md',
    'architecture.en.md': 'en/architecture.md',
    'assets.en.md': 'en/assets.md',
    'build.en.md': 'en/build.md',
    'cli.en.md': 'en/cli.md',
    'design-prototypes.en.md': 'en/design-prototypes.md',
    'example.en.md': 'en/example.md',
    'lifecycle.en.md': 'en/lifecycle.md',
    'schema.en.md': 'en/schema.md',
    'source.en.md': 'en/source.md',
    'target.en.md': 'en/target.md',
    'ui.en.md': 'en/ui.md',
    'validation.en.md': 'en/validation.md',
    'web.en.md': 'en/web.md'
  },

  locales: {
    root: zhTheme,
    en: enTheme
  },

  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'Cage',

    socialLinks: [
      { icon: 'github', link: 'https://github.com/cuihairu/cage' }
    ],

    footer: {
      message: '基于 Apache-2.0 许可发布',
      copyright: 'Copyright © 2026 cuihairu'
    }
  }
})
