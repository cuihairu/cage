// Cage 文档主题：默认主题 + 品牌配色与中文排印（custom.css 见同目录）
// + Mermaid 图表客户端渲染（Mermaid.vue，围栏替换见 config.mts）
import DefaultTheme from 'vitepress/theme'
import MermaidDiagram from './Mermaid.vue'
import './custom.css'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('MermaidDiagram', MermaidDiagram)
  },
}
