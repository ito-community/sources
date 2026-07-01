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
  archived?: boolean
  archived_reason?: string
  archived_date?: string
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
  <!-- Background matches iOS light mode default background (#f2f2f7) -->
  <div class="min-h-screen bg-[#f2f2f7] text-gray-900 font-sans selection:bg-gray-300 selection:text-black px-4 py-12 md:px-12 relative overflow-hidden">
    
    <!-- Abstract blurred blobs to highlight the glassmorphism effect -->
    <div class="absolute top-[-10%] left-[-10%] w-[60%] h-[60%] bg-white/60 rounded-full blur-[100px] pointer-events-none"></div>
    <div class="absolute bottom-[-10%] right-[-10%] w-[70%] h-[70%] bg-gray-300/40 rounded-full blur-[100px] pointer-events-none"></div>

    <header class="max-w-4xl mx-auto mb-12 relative z-10">
      <!-- Glassmorphic Header Card -->
      <div class="flex flex-col md:flex-row md:items-center md:justify-between gap-6 bg-white/50 backdrop-blur-2xl border border-white/60 shadow-xl shadow-black/5 rounded-[2rem] p-8">
        <div>
          <h1 class="text-3xl md:text-5xl font-bold tracking-tight text-black">
            {{ repo?.repo_name || 'Ito Repository' }}
          </h1>
          <p class="mt-2 text-base md:text-lg font-medium text-gray-600 max-w-xl leading-snug">
            {{ repo?.description || 'Browse and install plugins for Ito.' }}
          </p>
        </div>
        <a 
          v-if="repo"
          :href="addRepoUrl"
          class="inline-flex items-center justify-center bg-black/90 text-white px-6 py-3.5 text-sm font-semibold rounded-full shadow-md hover:bg-black active:scale-95 transition-all duration-200"
        >
          Add Repository
        </a>
      </div>
    </header>

    <main class="max-w-4xl mx-auto relative z-10">
      <div v-if="loading" class="flex justify-center items-center py-20">
        <div class="text-lg font-semibold text-gray-500 animate-pulse">Loading Plugins...</div>
      </div>
      <div v-else-if="error" class="bg-white/60 backdrop-blur-2xl border border-white/60 text-gray-800 font-medium rounded-3xl p-6 shadow-sm text-center">
        Error: {{ error }}
      </div>
      <div v-else-if="repo" class="grid grid-cols-1 sm:grid-cols-2 gap-6">
        <!-- Glassmorphic Plugin Cards -->
        <div 
          v-for="pkg in repo.packages" 
          :key="pkg.id"
          class="bg-white/40 backdrop-blur-2xl border border-white/60 shadow-lg shadow-black/5 hover:shadow-xl hover:shadow-black/10 rounded-[2rem] p-6 transition-all duration-300 flex flex-col justify-between group"
        >
          <div>
            <div class="flex items-center justify-between mb-5">
              <!-- iOS Style App Icon -->
              <div class="w-16 h-16 bg-white rounded-2xl shadow-sm border border-white overflow-hidden shrink-0 flex items-center justify-center group-hover:scale-105 transition-transform duration-300">
                <img 
                  v-if="pkg.icon_url" 
                  :src="pkg.icon_url" 
                  :alt="pkg.name"
                  class="w-full h-full object-cover"
                />
                <div v-else class="text-gray-400 font-semibold text-[10px] uppercase tracking-wider">
                  ICON
                </div>
              </div>
              <!-- Badges -->
              <div class="flex items-center gap-2">
                <span v-if="pkg.archived" class="text-xs font-semibold uppercase tracking-wider bg-orange-100/80 text-orange-700 border border-orange-200 px-3 py-1 rounded-full">
                  Archived
                </span>
                <span class="text-xs font-semibold uppercase tracking-wider bg-gray-200/50 text-gray-700 px-3 py-1 rounded-full">
                  {{ pkg.type }}
                </span>
              </div>
            </div>
            
            <h3 class="text-xl font-bold tracking-tight text-black mb-1">
              {{ normalizeName(pkg.name) }}
            </h3>
            <p class="text-sm font-medium text-gray-500" :class="{ 'mb-2': pkg.archived, 'mb-6': !pkg.archived }">
              Version {{ pkg.version }}
            </p>
            <p v-if="pkg.archived" class="text-xs font-medium text-orange-600/90 leading-snug mb-4">
              {{ pkg.archived_reason ? 'No longer maintained: ' + pkg.archived_reason : 'This plugin is no longer maintained.' }}
            </p>
          </div>

          <div class="mt-2 flex items-center justify-between pt-4 border-t border-gray-200/50">
            <span class="text-[11px] font-mono text-gray-400 truncate flex-1">
              {{ pkg.id }}
            </span>
          </div>
        </div>
      </div>
    </main>

    <footer class="max-w-4xl mx-auto mt-20 pb-8 text-center text-xs font-semibold text-gray-400 uppercase tracking-widest relative z-10">
      &copy; {{ new Date().getFullYear() }} &mdash; Powered by ito-pkg
    </footer>
  </div>
</template>

<style>
body {
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  background-color: #f2f2f7;
}
</style>
