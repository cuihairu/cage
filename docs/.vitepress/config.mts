import { defineConfig } from 'vitepress'

// Cage 文档站：设计稿（docs/design.md）+ 需求整理（docs/需求整理.md）拆分为七个栏目页。
// 原始底稿保留在 docs/ 下不入站（srcExclude），站内为拆分后的栏目内容。
export default defineConfig({
  lang: 'zh-CN',
  title: 'Cage',
  description: '游戏配置编译与验证框架——把异构配置源编译为经过验证、确定性构建的运行时配置资产',
  base: '/cage/',
  cleanUrls: true,
  lastUpdated: true,

  // favicon 不走自动 base，写全路径；导航 logo 见 themeConfig.logo（自动加 base）
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/cage/favicon.svg' }]
  ],

  // 原始设计稿与需求整理作为底稿保留，不进站点路由
  srcExclude: ['design.md', '需求整理.md'],

  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'Cage',

    nav: [
      { text: '首页', link: '/' },
      { text: '架构', link: '/architecture' },
      { text: 'Source', link: '/source' },
      { text: 'Schema', link: '/schema' },
      { text: 'Validation', link: '/validation' },
      { text: 'Target', link: '/target' },
      { text: 'CLI', link: '/cli' },
      { text: 'Build', link: '/build' },
      { text: 'Web', link: '/web' },
      { text: '完整示例', link: '/example' }
    ],

    sidebar: {
      '/': [
        {
          text: '概览',
          items: [
            { text: '架构', link: '/architecture' },
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
            { text: 'Web：Schema 编辑器', link: '/web' }
          ]
        }
      ]
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/cuihairu/cage' }
    ],

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

    editLink: {
      pattern: 'https://github.com/cuihairu/cage/edit/main/docs/:path',
      text: '在 GitHub 上编辑此页'
    },

    footer: {
      message: '基于 Apache-2.0 许可发布',
      copyright: 'Copyright © 2026 cuihairu'
    }
  }
})
