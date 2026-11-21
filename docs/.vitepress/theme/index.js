// Cage 文档主题：默认主题 + 品牌配色与中文排印（custom.css 见同目录）
// + Mermaid 图表客户端渲染（Mermaid.vue，围栏替换见 config.mts）
// + 浏览器语言自动跳转（lang-redirect.js：仅根页、整页加载一次、手动切换记忆）
import DefaultTheme from 'vitepress/theme'
import MermaidDiagram from './Mermaid.vue'
import { installLangRedirect } from './lang-redirect'
import './custom.css'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('MermaidDiagram', MermaidDiagram)
    installLangRedirect()
  },
}
