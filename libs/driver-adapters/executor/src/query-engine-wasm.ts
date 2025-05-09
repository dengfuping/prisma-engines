import fs from 'node:fs/promises'
import path from 'node:path'
import { __dirname, normaliseProvider } from './utils'
import type { SqlQueryable } from '@prisma/driver-adapter-utils'

const relativePath = '../../../../query-engine/query-engine-wasm/pkg'

const initializedModules = new Set<SqlQueryable['provider']>()

export async function getQueryEngineForProvider(
  provider: SqlQueryable['provider'],
) {
  const normalisedProvider = normaliseProvider(provider)
  const engine = await import(
    `${relativePath}/${normalisedProvider}/query_engine_bg.js`
  )

  if (!initializedModules.has(provider)) {
    const bytes = await fs.readFile(
      path.resolve(
        __dirname,
        relativePath,
        normalisedProvider,
        'query_engine_bg.wasm',
      ),
    )

    const module = new WebAssembly.Module(bytes)
    const instance = new WebAssembly.Instance(module, {
      './query_engine_bg.js': engine,
    })
    const wbindgen_start = instance.exports.__wbindgen_start as () => void
    engine.__wbg_set_wasm(instance.exports)
    wbindgen_start()
    initializedModules.add(provider)
  }

  return engine.QueryEngine
}
