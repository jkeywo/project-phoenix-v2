// Build receipts are written only after a successful build from clean source.
// The runner verifies the frozen executable, symbols, content and bundle again.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export function fileHash(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

export function treeHash(directory) {
  const hash = crypto.createHash('sha256');
  function visit(relative) {
    for (const name of fs.readdirSync(path.join(directory, relative)).sort()) {
      const child = path.join(relative, name);
      const absolute = path.join(directory, child);
      if (fs.statSync(absolute).isDirectory()) visit(child);
      else hash.update(child.replaceAll('\\', '/') + '\0' + fileHash(absolute) + '\n');
    }
  }
  visit('');
  return hash.digest('hex');
}

function git(root, args) {
  const result = spawnSync('git', ['-C', root, ...args], { encoding: 'utf8' });
  if (result.status) throw new Error(result.stderr);
  return result.stdout.trim();
}

export function verifyReceipt(receipt, root, binary, bundle) {
  return receipt.sourceRevision === git(root, ['rev-parse', 'HEAD'])
    && !git(root, ['status', '--porcelain', '--untracked-files=normal'])
    && receipt.binarySha256 === fileHash(binary)
    && (!receipt.pdbSha256 || receipt.pdbSha256 === fileHash(path.join(path.dirname(binary), receipt.pdbFile)))
    && Object.entries(receipt.dllSha256 || {}).every(([name, hash]) => fileHash(path.join(path.dirname(binary), name)) === hash)
    && (receipt.runtime !== 'native' || (receipt.sdkResourcesSha256
      && receipt.sdkResourcesSha256 === treeHash(path.join(path.dirname(binary), 'sdk-resources'))))
    && receipt.contentSha256 === treeHash(path.join(root, 'assets'))
    && (!bundle || receipt.bundleSha256 === treeHash(bundle));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [mode, rootArg, outputArg, runtime = 'native', bundleArg] = process.argv.slice(2);
  if (!['build', 'verify', 'hash'].includes(mode) || !rootArg) {
    throw new Error('Usage: profile-provenance.mjs build <root> <new-output-dir> <native|headless|attribution> [bundle] | verify <root> <receipt> [binary] [bundle] | hash <directory>');
  }
  const root = path.resolve(rootArg);
  if (mode === 'hash') console.log(treeHash(root));
  else if (mode === 'verify') {
    const receipt = JSON.parse(fs.readFileSync(outputArg, 'utf8').replace(/^\uFEFF/, ''));
    const ok = verifyReceipt(receipt, root, path.resolve(runtime), bundleArg && path.resolve(bundleArg));
    console.log(JSON.stringify({ verified: ok }));
    if (!ok) process.exitCode = 2;
  } else {
    if (!['native', 'headless', 'attribution'].includes(runtime)) throw new Error('Unknown runtime');
    if (git(root, ['status', '--porcelain', '--untracked-files=normal'])) throw new Error('Commit source changes before building a comparison artifact');
    const output = path.resolve(outputArg);
    if (fs.existsSync(output)) throw new Error('Output directory already exists');
    const revision = git(root, ['rev-parse', 'HEAD']);
    const bin = { native: 'phoenix-host', headless: 'phoenix-headless', attribution: 'profile_systems' }[runtime];
    const profile = { native: 'measure', headless: 'release', attribution: 'attribution' }[runtime];
    const features = { native: 'host,ultralight', headless: 'headless', attribution: 'host,headless,perf' }[runtime];
    // A comparison artifact must come from this checkout. Other worktrees can
    // overwrite a shared target's package outputs while this build waits for
    // its lock; file timestamps alone cannot establish source provenance.
    const targetDirectory = path.join(root, '.phoenix', 'profiling-target');
    const args = ['build', '--profile', profile, '--features', features,
      runtime === 'attribution' ? '--example' : '--bin', bin,
      '--target-dir', targetDirectory, '--message-format=json-render-diagnostics'];
    // Cargo reports the build-script output that this exact build used, even
    // when it is cached. Never guess among stale target/build SDK directories.
    const build = spawnSync('cargo', args, { cwd: root, encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 64 * 1024 * 1024 });
    if (build.status !== 0) throw new Error('Build failed');
    const messages = build.stdout.split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line));
    const sdkOutputs = messages.filter(message => message.reason === 'build-script-executed'
      && message.package_id.includes('ul-next-sys')).map(message => path.join(message.out_dir, 'ul-sdk'));
    if (runtime === 'native' && sdkOutputs.length !== 1) throw new Error('Expected exactly one Ultralight SDK build output');
    if (git(root, ['rev-parse', 'HEAD']) !== revision || git(root, ['status', '--porcelain', '--untracked-files=normal'])) {
      throw new Error('Source changed during build');
    }
    const binaryArtifact = messages.find(message => message.reason === 'compiler-artifact'
      && message.target.name === bin && message.executable);
    if (!binaryArtifact) throw new Error('Cargo did not report the requested executable');
    fs.mkdirSync(output, { recursive: true });
    const executable = path.basename(binaryArtifact.executable);
    fs.copyFileSync(binaryArtifact.executable, path.join(output, executable));
    // Only freeze symbols Cargo reports for this artifact, never an old PDB
    // left beside a stripped executable by an earlier build configuration.
    const emittedPdb = binaryArtifact.filenames.find(file => file.toLowerCase().endsWith('.pdb'));
    const pdbFile = emittedPdb ? path.basename(emittedPdb) : null;
    if (emittedPdb) fs.copyFileSync(emittedPdb, path.join(output, pdbFile));
    if (runtime === 'native') {
      const sdk = sdkOutputs[0];
      const sdkBin = path.join(sdk, 'bin');
      for (const name of ['AppCore.dll', 'Ultralight.dll', 'UltralightCore.dll', 'WebCore.dll']) {
        if (!fs.existsSync(path.join(sdkBin, name))) throw new Error(`Required SDK library missing: ${name}`);
      }
      for (const name of fs.readdirSync(sdkBin).filter(name => name.endsWith('.dll'))) {
        fs.copyFileSync(path.join(sdkBin, name), path.join(output, name));
      }
      fs.cpSync(path.join(sdk, 'resources'), path.join(output, 'sdk-resources'), { recursive: true });
    }
    const pdb = pdbFile && path.join(output, pdbFile);
    if (runtime === 'attribution' && process.platform === 'win32' && !pdb) {
      throw new Error('Attribution build must retain its PDB');
    }
    const receipt = {
      sourceRevision: revision, sourceClean: true, runtime, profile, features,
      command: ['cargo', ...args], createdUtc: new Date().toISOString(),
      executable: path.join(output, executable), binarySha256: fileHash(path.join(output, executable)),
      pdbFile, pdbSha256: pdb ? fileHash(pdb) : null,
      dllSha256: Object.fromEntries(fs.readdirSync(output).filter(name => name.endsWith('.dll'))
        .map(name => [name, fileHash(path.join(output, name))])),
      sdkResourcesSha256: runtime === 'native' ? treeHash(path.join(output, 'sdk-resources')) : null,
      contentSha256: treeHash(path.join(root, 'assets')),
      bundleSha256: bundleArg ? treeHash(path.resolve(bundleArg)) : null,
    };
    fs.writeFileSync(path.join(output, 'build-receipt.json'), JSON.stringify(receipt, null, 2) + '\n');
  }
}
