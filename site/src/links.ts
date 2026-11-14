export const REPO = 'https://github.com/jramirez-codes/nex'
export const RELEASES = `${REPO}/releases/latest`
export const BENCH_DOC = `${REPO}/blob/main/bench/README.md`
/** Resolves a file in public/ against Vite's base path. */
export const asset = (path: string) => `${import.meta.env.BASE_URL}${path}`
