<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'

interface Package {
  id: string
  name: string
  version: string
  min_app_version: string
  download_url: string
  icon_url: string | null
  sha256: string
  type: string
}

interface RepoIndex {
  repo_name: string
  repo_url: string
  description: string
  packages: Package[]
}

const repo = ref<RepoIndex | null>(null)
const loading = ref(true)
const error = ref<string | null>(null)

function normalizeName(name: string) {
  // Normalize names like "VIOLETSCANS" to "Violetscans"
  // If it's already mixed case or has spaces, keep it mostly as is but clean it up
  if (name === name.toUpperCase()) {
    return name.charAt(0) + name.slice(1).toLowerCase()
  }
  return name
}

const addRepoUrl = computed(() => {
  if (!repo.value) return '#'
  return `ito://repo/add?url=${encodeURIComponent(repo.value.repo_url + '/index.json')}`
})

onMounted(async () => {
  try {
    // index.json is expected to be in the same directory as index.html after build
    const response = await fetch('./index.json')
    if (!response.ok) throw new Error('Failed to fetch index.json')
    repo.value = await response.json()
  } catch (err) {
    error.value = err instanceof Error ? err.message : 'An unknown error occurred'
    console.error(err)
  } finally {
    loading.value = false
  }
})
</script>

<template>
  <div class="min-h-screen bg-white text-black font-sans selection:bg-black selection:text-white px-6 py-12 md:px-12">
    <header class="max-w-4xl mx-auto mb-16 border-b border-black pb-8">
      <div class="flex flex-col md:flex-row md:items-end md:justify-between gap-6">
        <div>
          <h1 class="text-4xl md:text-6xl font-black tracking-tighter uppercase italic">
            {{ repo?.repo_name || 'Ito Repository' }}
          </h1>
          <p class="mt-4 text-lg md:text-xl font-medium opacity-60 max-w-2xl leading-relaxed">
            {{ repo?.description || 'Browse and install plugins for Ito.' }}
          </p>
        </div>
        <a 
          v-if="repo"
          :href="addRepoUrl"
          class="inline-block bg-black text-white px-8 py-4 text-sm font-bold uppercase tracking-widest hover:bg-white hover:text-black border-2 border-black transition-colors duration-300"
        >
          Add Repository
        </a>
      </div>
    </header>

    <main class="max-w-4xl mx-auto">
      <div v-if="loading" class="text-xl font-bold animate-pulse">LOADING...</div>
      <div v-else-if="error" class="text-red-600 font-bold border-2 border-red-600 p-4">
        ERROR: {{ error }}
      </div>
      <div v-else-if="repo" class="grid grid-cols-1 md:grid-cols-2 gap-8">
        <div 
          v-for="pkg in repo.packages" 
          :key="pkg.id"
          class="group border border-black p-6 hover:bg-black hover:text-white transition-all duration-300 flex flex-col justify-between"
        >
          <div>
            <div class="flex items-start justify-between mb-6">
              <div class="w-16 h-16 border border-black group-hover:border-white overflow-hidden bg-white shrink-0">
                <img 
                  v-if="pkg.icon_url" 
                  :src="pkg.icon_url" 
                  :alt="pkg.name"
                  class="w-full h-full object-cover"
                />
                <div v-else class="w-full h-full flex items-center justify-center text-black font-bold text-xs uppercase p-2 text-center">
                  No Icon
                </div>
              </div>
              <span class="text-[10px] font-bold uppercase tracking-widest border border-black group-hover:border-white px-2 py-1">
                {{ pkg.type }}
              </span>
            </div>
            
            <h3 class="text-2xl font-black uppercase tracking-tight leading-none mb-2">
              {{ normalizeName(pkg.name) }}
            </h3>
            <p class="text-xs font-bold opacity-60 mb-6 uppercase tracking-widest">
              Version {{ pkg.version }}
            </p>
          </div>

          <div class="mt-4 flex items-center justify-between gap-4">
            <span class="text-[10px] font-mono opacity-40 truncate flex-1 uppercase tracking-tighter">
              {{ pkg.id }}
            </span>
          </div>
        </div>
      </div>
    </main>

    <footer class="max-w-4xl mx-auto mt-24 pt-8 border-t border-black text-[10px] font-bold uppercase tracking-[0.2em] opacity-40">
      &copy; {{ new Date().getFullYear() }} &mdash; POWERED BY ITO-PKG
    </footer>
  </div>
</template>

<style>
/* Base typography resets if needed, though Tailwind 4 handles most */
body {
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}
</style>
