#!/usr/bin/env node
// Claude Code Statusline - SkyNexus AI Edition
// Format: SkyNexus AI Corp. │ model │ task (if active) │ directory progress-bar percentage
// Also writes sideband status for NCC ContextBar and chains to original statusline.
const fs = require('fs');
const path = require('path');
const os = require('os');
const { execSync, spawn } = require('child_process');

const SKYNEXUS = '\x1b[38;5;153mSkyNexus AI Corp.\x1b[0m';

function readOriginalStatuslineCmd() {
  try {
    const settingsPath = path.join(os.homedir(), '.claude', 'settings.json');
    const settings = JSON.parse(fs.readFileSync(settingsPath, 'utf8'));
    const cmd = settings?.statusLine?.command;
    if (cmd && !cmd.includes('nexus-statusline.')) return cmd;
  } catch (e) {}
  return null;
}

function appDataDir() {
  if (process.env.NCC_DATA_DIR) return process.env.NCC_DATA_DIR;
  switch (process.platform) {
    case 'win32':
      return path.join(process.env.APPDATA || path.join(os.homedir(), 'AppData', 'Roaming'), 'NexusControlPlane');
    case 'darwin':
      return path.join(os.homedir(), 'Library', 'Application Support', 'NexusControlPlane');
    default:
      return path.join(process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share'), 'NexusControlPlane');
  }
}

let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => input += chunk);
process.stdin.on('end', () => {
  try {
    const data = JSON.parse(input);

    // Fire-and-forget telemetry emitter (writes JSONL + posts turn_complete event to ledger).
    // Falls through silently if emitter is not installed on this host.
    try {
      const emitterPath = path.join(os.homedir(), '.claude', 'hooks', 'telemetry-emitter.sh');
      if (fs.existsSync(emitterPath)) {
        const em = spawn(emitterPath, [], { stdio: ['pipe', 'ignore', 'ignore'], detached: true });
        em.on('error', () => {});
        em.stdin.on('error', () => {});
        em.stdin.write(input);
        em.stdin.end();
        em.unref();
      }
    } catch (e) { /* never break the statusline */ }

    const sessionId = process.env.NCC_SESSION_ID;
    const model = data.model?.display_name || 'Claude';
    const dir = path.basename(data.workspace?.current_dir || process.cwd());
    const session = data.session_id || '';
    const remaining = data.context_window?.remaining_percentage;

    // Write sideband status file for NCC ContextBar (always, before any rendering)
    if (sessionId) {
      const statusDir = path.join(appDataDir(), 'status');
      fs.mkdirSync(statusDir, { recursive: true });
      const statusFile = path.join(statusDir, `${sessionId}.json`);
      fs.writeFileSync(statusFile, JSON.stringify(data, null, 2));
    }

    // Always delegate to the original statusline (user's skynexus-statusline.js)
    // for rendering — it has the correct brand color, rate-limit bars, and features.
    // NCC only adds the sideband write above.
    const originalCmd = process.env.NCC_ORIGINAL_STATUSLINE || readOriginalStatuslineCmd();
    if (originalCmd) {
      try {
        const output = execSync(originalCmd, { input, encoding: 'utf8', timeout: 5000 });
        process.stdout.write(output);
        return;
      } catch (e) { /* fall through to fallback output */ }
    }

    // Fallback: render our own output only if original statusline is unavailable
    // Context window display (scaled to 80% limit — 80% real = 100% displayed)
    let ctx = '';
    if (remaining != null) {
      const rawUsed = Math.max(0, Math.min(100, 100 - Math.round(remaining)));
      const used = Math.min(100, Math.round((rawUsed / 80) * 100));
      const filled = Math.floor(used / 10);
      const bar = '█'.repeat(filled) + '░'.repeat(10 - filled);
      if (used < 63) {
        ctx = ` \x1b[32m${bar} ${used}%\x1b[0m`;
      } else if (used < 81) {
        ctx = ` \x1b[33m${bar} ${used}%\x1b[0m`;
      } else if (used < 95) {
        ctx = ` \x1b[38;5;208m${bar} ${used}%\x1b[0m`;
      } else {
        ctx = ` \x1b[5;31m💀 ${bar} ${used}%\x1b[0m`;
      }
    }

    // Current task from todos
    let task = '';
    const homeDir = os.homedir();
    const todosDir = path.join(homeDir, '.claude', 'todos');
    const taskSession = session || sessionId || '';
    if (taskSession && fs.existsSync(todosDir)) {
      const files = fs.readdirSync(todosDir)
        .filter(f => f.startsWith(taskSession) && f.includes('-agent-') && f.endsWith('.json'))
        .map(f => ({ name: f, mtime: fs.statSync(path.join(todosDir, f)).mtime }))
        .sort((a, b) => b.mtime - a.mtime);

      if (files.length > 0) {
        try {
          const todos = JSON.parse(fs.readFileSync(path.join(todosDir, files[0].name), 'utf8'));
          const inProgress = todos.find(t => t.status === 'in_progress');
          if (inProgress) task = inProgress.activeForm || '';
        } catch (e) {}
      }
    }

    // Assemble output
    if (task) {
      process.stdout.write(`${SKYNEXUS} │ \x1b[2m${model}\x1b[0m │ \x1b[1m${task}\x1b[0m │ \x1b[2m${dir}\x1b[0m${ctx}`);
    } else {
      process.stdout.write(`${SKYNEXUS} │ \x1b[2m${model}\x1b[0m │ \x1b[2m${dir}\x1b[0m${ctx}`);
    }
  } catch (e) {
    // Silent fail — don't break statusline on parse errors
  }
});
