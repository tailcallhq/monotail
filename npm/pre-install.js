const os = process.platform
const arch = process.arch
const libc = process.libc

const dependency = libc ? Object.keys(optionalDependencies).find((name) => name.includes(`${os}-${arch}-${libc}`)) : Object.keys(optionalDependencies).find((name) => name.includes(`${os}-${arch}`))
if (!dependency) {
  const redColor = "\x1b[31m"
  const resetColor = "\x1b[0m"
  console.error(`${redColor} Tailcall does not support platform ${os} arch ${arch} ${resetColor}`)
  process.exit(1)
}
