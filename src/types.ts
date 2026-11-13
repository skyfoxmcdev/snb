export type Lang = "fr" | "en";
export type Localized = string | { en?: string; fr?: string };

export interface Settings {
  schemaVersion: number;
  username: string;
  gameDir: string;
  javaPath: string;
  language: string;
  theme: string;
  minMemoryMb: number;
  maxMemoryMb: number;
  width: number;
  height: number;
  fullscreen: boolean;
  jvmArgs: string;
  gameArgs: string;
  autoInstallJava: boolean;
  autoBackup: boolean;
  backupIntervalHours: number;
  backupRetention: number;
  lastBackupMs: number;
  onboardingComplete: boolean;
}

export interface ServerStatus {
  state: string;
  playersOnline?: number;
  playersMax?: number;
  motd?: string;
  latencyMs?: number;
}

export interface Readiness {
  ready: boolean;
  issues: string[];
  components: { id: string; status: string; code?: string }[];
}

export interface UpdateCheck {
  required: boolean;
  error?: string;
  decision?: {
    currentVersion: string;
    latestVersion: string;
    mandatory: boolean;
    minimumVersion: string;
    notes?: Localized;
    artifact: { fileName: string; size: number };
  };
}

export interface Bootstrap {
  launcherVersion: string;
  settings: Settings;
  remote: any;
  readiness: Readiness;
  serverStatus: ServerStatus;
  gameRunning: boolean;
  paths: { gameDir: string; logsDir: string; backupsDir: string; modsDir: string };
  runtime: {
    minecraft: { version: string };
    neoforge: { version: string };
    javaMajor: number;
    automodpack: { version: string };
  };
  update: UpdateCheck;
  updateRunId?: string;
}

export interface StepProgress {
  current: number;
  total: number;
  filesDone: number;
  filesTotal: number;
  unit: string;
}

export interface InstallStep {
  id: string;
  status: string;
  progress?: StepProgress;
  warning?: string;
}

export interface InstallReport {
  mode: string;
  running: boolean;
  ready: boolean;
  steps: InstallStep[];
  currentStep?: string;
  error?: { code: string; detail?: string };
  warnings: { code: string }[];
  finishedAtMs?: number;
}

export interface ModView {
  fileName: string;
  enabled: boolean;
  size: number;
  managed: boolean;
  version?: string;
  name?: string;
  modId?: string;
  authors?: string;
  description?: string;
  icon?: string;
}

export interface ContentView {
  name: string;
  size: number;
  directory: boolean;
}

export interface Backup {
  fileName: string;
  createdAtMs: number;
  size: number;
  reason: string;
}

export type MojangState = "operational" | "degraded" | "unavailable" | "unknown";
