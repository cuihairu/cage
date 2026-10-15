<script setup>
// Mermaid 图表客户端渲染组件：markdown 围栏 ```mermaid 由 config.mts 的
// fence 钩子替换为本组件标签，这里在浏览器端把源码渲染成 SVG。
// - 仅客户端动态 import mermaid，SSG 产物零负担
// - 监听 <html> 的 class 变化：暗色模式切换时用对应主题重渲染
import { ref, watch, onMounted, onBeforeUnmount } from 'vue'

const props = defineProps({
  code: { type: String, required: true },
})

const svg = ref('')
const failed = ref(false)
let observer = null
let renderSeq = 0

async function render() {
  const seq = ++renderSeq
  try {
    const mermaid = (await import('mermaid')).default
    const dark = document.documentElement.classList.contains('dark')
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: 'strict',
      theme: dark ? 'dark' : 'neutral',
      fontFamily: 'inherit',
      // SVG text labels: node boxes are sized from measured text metrics, so
      // multi-line labels can never be cut off by the node border. With the
      // default htmlLabels (foreignObject divs) the box is sized from an
      // estimate; when a label wraps to more lines than estimated (long
      // <br/> segments, CJK fonts), the extra lines are clipped top and bottom.
      htmlLabels: false,
    })
    const id = `mermaid-svg-${seq}-${Date.now().toString(36)}`
    const { svg: out } = await mermaid.render(id, props.code)
    if (seq === renderSeq) {
      svg.value = out
      failed.value = false
    }
  } catch {
    // 语法错误或渲染失败：回退展示源码，图表坏了不许吞掉内容
    if (seq === renderSeq) {
      failed.value = true
      svg.value = ''
    }
  }
}

onMounted(() => {
  render()
  observer = new MutationObserver(render)
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['class'],
  })
})

onBeforeUnmount(() => observer?.disconnect())
watch(() => props.code, render)
</script>

<template>
  <div v-if="!failed" class="mermaid-diagram" v-html="svg" />
  <pre v-else class="mermaid-fallback"><code>{{ code }}</code></pre>
</template>
