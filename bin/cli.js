#!/usr/bin/env node

/**
 * magic-cursor CLI & Setup Wizard
 * Distribution wrapper for Magic Cursor (Windows Native Rust Daemon)
 */

const fs = require('fs');
const path = require('path');
const os = require('os');
const { spawn, execSync } = require('child_process');
const readline = require('readline');

const BANNER = `
\x1b[36m  __  __             _         _____                            
 |  \\/  |           (_)       / ____|                           
 | \\  / | __ _  __ _ _  ___  | |    _   _ _ __ ___  ___  _ __   
 | |\\/| |/ _\` |/ _\` | |/ __| | |   | | | | '__/ __|/ _ \\| '__|  
 | |  | | (_| | (_| | | (__  | |___| |_| | |  \\__ \\ (_) | |     
 |_|  |_|\\__,_|\\__, |_|\\___|  \\_____\\__,_|_|  |___/\\___/|_|     
                __/ |                                           
               |___/   \x1b[35m[Low-Resource Windows Magic Pointer]\x1b[0m
`;

function getAppDataDir() {
  const appData = process.env.APPDATA || path.join(os.homedir(), 'AppData', 'Roaming');
  return path.join(appData, 'MagicCursor');
}

function getConfigPath() {
  return path.join(getAppDataDir(), 'config.toml');
}

function getBinaryPath() {
  // Check workspace target/release/magic-cursor.exe
  const localTarget = path.join(__dirname, '..', 'target', 'release', 'magic-cursor.exe');
  if (fs.existsSync(localTarget)) {
    return localTarget;
  }
  const localDev = path.join(__dirname, '..', 'target', 'debug', 'magic-cursor.exe');
  if (fs.existsSync(localDev)) {
    return localDev;
  }
  const appDataBin = path.join(getAppDataDir(), 'bin', 'magic-cursor.exe');
  if (fs.existsSync(appDataBin)) {
    return appDataBin;
  }
  return localTarget;
}

function prompt(rl, question, defaultValue) {
  return new Promise((resolve) => {
    const q = defaultValue !== undefined ? `${question} \x1b[90m(${defaultValue})\x1b[0m: ` : `${question}: `;
    rl.question(q, (answer) => {
      resolve(answer.trim() || defaultValue);
    });
  });
}

async function runSetup() {
  console.log(BANNER);
  console.log('\x1b[1m\x1b[32m✨ Welcome to the magic-cursor Setup Wizard!\x1b[0m');
  console.log('Replicating Google Magic Pointer with native Rust performance.\n');

  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  });

  const configDir = getAppDataDir();
  const configPath = getConfigPath();
  if (!fs.existsSync(configDir)) {
    fs.mkdirSync(configDir, { recursive: true });
  }

  console.log('\x1b[33m--- [1/4] LLM Provider Configuration ---\x1b[0m');
  console.log('Supported: gemini (Google), openai (OpenAI/vLLM), anthropic (Claude), ollama (Local)');
  const provider = (await prompt(rl, 'Select LLM Provider', 'gemini')).toLowerCase();

  let defaultModel = 'gemini-2.0-flash';
  let defaultEndpoint = 'https://generativelanguage.googleapis.com/v1beta';

  if (provider === 'openai') {
    defaultModel = 'gpt-4o';
    defaultEndpoint = 'https://api.openai.com/v1/chat/completions';
  } else if (provider === 'anthropic' || provider === 'claude') {
    defaultModel = 'claude-3-5-sonnet-20241022';
    defaultEndpoint = 'https://api.anthropic.com/v1/messages';
  } else if (provider === 'ollama') {
    defaultModel = 'qwen2.5-vl:latest';
    defaultEndpoint = 'http://localhost:11434/v1/chat/completions';
  }

  const model = await prompt(rl, 'Model Identifier', defaultModel);
  const endpoint = await prompt(rl, 'API Endpoint', defaultEndpoint);
  const apiKey = await prompt(rl, 'API Key (leave blank for local Ollama)', '');

  console.log('\n\x1b[33m--- [2/4] Activation & Triggers ---\x1b[0m');
  const hotkey = await prompt(rl, 'Global Shortcut', 'Win+Shift+G');
  const activationMode = await prompt(rl, 'Trigger Mode (both / hotkey / wiggle)', 'both');
  const wiggleSensitivity = parseInt(await prompt(rl, 'Mouse Wiggle Sensitivity (1-10)', '5'), 10);

  console.log('\n\x1b[33m--- [3/4] REA (Reverse Engineer Anything) MCP Integration ---\x1b[0m');
  const enableRea = (await prompt(rl, 'Enable REA MCP context enrichment (y/n)?', 'y')).toLowerCase().startsWith('y');
  const reaCommand = enableRea ? await prompt(rl, 'REA command/package', 'npx -y rea-agents') : 'npx -y rea-agents';

  rl.close();

  // Generate config.toml
  const tomlContent = `[general]
activation_mode = "${activationMode}"
hotkey = "${hotkey}"
wiggle_sensitivity = ${wiggleSensitivity}
wiggle_window_ms = 300
wiggle_cooldown_ms = 1200

[api]
provider = "${provider}"
api_key = "${apiKey}"
endpoint = "${endpoint}"
model = "${model}"
system_prompt = "You are MagicCursor, a high-speed desktop AI assistant operating under the user's cursor. Provide concise, clear, and actionable responses."
temperature = 0.7
max_tokens = 2048

[rea]
enabled = ${enableRea}
mcp_command = "${reaCommand}"
auto_enrich = true

[overlay]
width = 560.0
height = 460.0
crop_width = 600
crop_height = 400
auto_capture_screen = true
acrylic_blur = true
dark_mode = true
`;

  fs.writeFileSync(configPath, tomlContent, 'utf8');
  console.log(`\n\x1b[32m✔ Configuration saved to:\x1b[0m ${configPath}`);

  console.log('\n\x1b[33m--- [4/4] Building Rust Core Daemon ---\x1b[0m');
  try {
    console.log('Compiling release binary with Cargo...');
    const projectRoot = path.join(__dirname, '..');
    execSync('cargo build --release', { cwd: projectRoot, stdio: 'inherit' });
    console.log('\n\x1b[32m✔ Build complete!\x1b[0m');
  } catch (err) {
    console.warn('\x1b[31mNotice: Cargo build returned non-zero. If pre-built binary is present, continuing.\x1b[0m');
  }

  console.log('\n\x1b[1m\x1b[36m🎉 magic-cursor is ready!\x1b[0m');
  console.log('To start in background, run:');
  console.log('  \x1b[32mnpx @its-jani/magic-cursor start\x1b[0m');
  console.log('\nHotkeys active: \x1b[1m' + hotkey + '\x1b[0m (Fallback: Ctrl+Alt+G) or rapid mouse wiggle.');
}

function startDaemon() {
  const binary = getBinaryPath();
  if (!fs.existsSync(binary)) {
    console.error(`\x1b[31mError: magic-cursor binary not found at ${binary}.\x1b[0m`);
    console.log('Please run: \x1b[32mnpx @its-jani/magic-cursor setup\x1b[0m');
    process.exit(1);
  }

  console.log(`Launching magic-cursor daemon: ${binary}`);
  const child = spawn(binary, [], {
    detached: true,
    stdio: 'ignore',
    windowsHide: true,
  });

  child.unref();
  console.log(`\x1b[32m✔ magic-cursor daemon running in background (PID: ${child.pid}).\x1b[0m`);
  console.log('Check your Windows System Tray for the Magic Cursor icon.');
}

function stopDaemon() {
  try {
    execSync('taskkill /F /IM magic-cursor.exe', { stdio: 'ignore' });
    console.log('\x1b[32m✔ Stopped magic-cursor daemon.\x1b[0m');
  } catch (err) {
    console.log('\x1b[33mNo running magic-cursor processes found.\x1b[0m');
  }
}

function checkStatus() {
  try {
    const output = execSync('tasklist /FI "IMAGENAME eq magic-cursor.exe" /NH', { encoding: 'utf8' });
    if (output.includes('magic-cursor.exe')) {
      console.log('\x1b[32m● magic-cursor daemon is RUNNING in background.\x1b[0m');
    } else {
      console.log('\x1b[33m○ magic-cursor daemon is NOT running.\x1b[0m');
      console.log('Run \x1b[32mnpx @its-jani/magic-cursor start\x1b[0m to launch.');
    }
  } catch (err) {
    console.log('\x1b[31mError checking process status.\x1b[0m');
  }
}

function openConfig() {
  const configPath = getConfigPath();
  if (fs.existsSync(configPath)) {
    console.log(`Opening config at: ${configPath}`);
    execSync(`start notepad "${configPath}"`, { shell: true });
  } else {
    console.log('Config not found. Running setup...');
    runSetup();
  }
}

// =========================================================================
// CLI Router
// =========================================================================

const args = process.argv.slice(2);
const command = args[0] || 'help';

switch (command.toLowerCase()) {
  case 'setup':
  case 'init':
    runSetup();
    break;
  case 'start':
  case 'run':
    startDaemon();
    break;
  case 'stop':
  case 'kill':
    stopDaemon();
    break;
  case 'status':
    checkStatus();
    break;
  case 'config':
    openConfig();
    break;
  default:
    console.log(BANNER);
    console.log(`Usage:
  npx @its-jani/magic-cursor setup     Run interactive setup wizard
  npx @its-jani/magic-cursor start     Launch background daemon
  npx @its-jani/magic-cursor status    Check if background daemon is running
  npx @its-jani/magic-cursor stop      Stop running background daemon
  npx @its-jani/magic-cursor config    Open config.toml in editor
`);
    break;
}
