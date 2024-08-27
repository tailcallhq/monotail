import * as fs from "fs/promises"
import {resolve, dirname} from "path"
import * as yml from "yaml"
import {fileURLToPath} from "url"
import {parse} from "ts-command-line-args"
import {PackageJson as IPackageJSON} from "type-fest"

const __dirname = dirname(fileURLToPath(import.meta.url))

interface ICLI {
  version: string
  name: string
}

const options = parse<ICLI>({
  version: {alias: "v", type: String},
  name: {alias: "n", type: String},
})

async function genServerPackage() {
  const packageVersion = options.version || "0.1.0"
  const name = options.name || "@tailcallhq/tailcall"

  console.log(`Generating package.json with version ${packageVersion}`)

  const packageJson = await fs.readFile(resolve(__dirname, "./package.json"), "utf8")
  const basePackage = JSON.parse(packageJson) as IPackageJSON
  const {description, license, repository, homepage, keywords} = basePackage

  const tailcallPackage: IPackageJSON = {
    description: description!,
    license: license!,
    repository: repository!,
    homepage: homepage!,
    keywords: keywords!,
    name: name,
    type: "module",
    version: packageVersion,
    scarfSettings: {
      defaultOptIn: true,
      allowTopLevel: true,
    },
    dependencies: {
      "detect-libc": "^2.0.2",
      "@scarf/scarf": "^1.3.0",
      yaml: "^2.3.3",
      axios: "^1.7.4",
    },
    scripts: {
      postinstall: "node ./scripts/post-install.js",
      preinstall: "node ./scripts/pre-install.js",
    },
    bin: {
      tailcall: "bin/tailcall", // will replace with respective platform binary later.
    },
  }

  // Define the directory path where the package.json should be created
  const directoryPath = resolve(__dirname, "@tailcallhq/tailcall")
  const scriptsPath = resolve(directoryPath, "./scripts")
  const binPath = resolve(directoryPath, "./bin")

  await fs.mkdir(scriptsPath, {recursive: true})
  await fs.mkdir(binPath, {recursive: true})
  await fs.mkdir(directoryPath, {recursive: true})

  const postInstallScript = await fs.readFile(resolve(__dirname, "./post-install.js"), "utf8")
  const preInstallScript = await fs.readFile(resolve(__dirname, "./pre-install.js"), "utf8")
  const utilsScript = await fs.readFile(resolve(__dirname, "./utils.js"), "utf8")
  const buildMatrix = await fs.readFile(resolve(__dirname, "../.github/build-matrix.yaml"), "utf8")

  const postInstallScriptContent = `const version = "${packageVersion}";\n${postInstallScript}`

  await fs.writeFile(resolve(scriptsPath, "post-install.js"), postInstallScriptContent, "utf8")
  await fs.writeFile(resolve(scriptsPath, "pre-install.js"), preInstallScript, "utf8")
  await fs.writeFile(resolve(scriptsPath, "utils.js"), utilsScript, "utf8")
  await fs.writeFile(resolve(directoryPath, "./build-matrix.yaml"), buildMatrix, "utf8")

  await fs.writeFile(resolve(directoryPath, "./package.json"), JSON.stringify(tailcallPackage, null, 2), "utf8")

  await fs.copyFile(resolve(__dirname, "../README.md"), resolve(directoryPath, "./README.md"))
}

await genServerPackage()
