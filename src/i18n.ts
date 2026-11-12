import type { Lang, Localized } from "./types";

export const strings: Record<Lang, Record<string, string>> = {
  fr: {
    play: "Jouer", install: "Installer", installing: "Installation…", launching: "Lancement…",
    online: "En ligne", offline: "Hors ligne", unknown: "Statut inconnu", maintenance: "Maintenance",
    settings: "Paramètres", mods: "Mods", backups: "Sauvegardes", diagnostics: "Diagnostic",
    welcome: "Bienvenue dans Sky & Beyond", intro: "Le ciel n'est qu'un début. Choisissez votre pseudo et votre dossier, le launcher prépare tout.",
    username: "Pseudo Minecraft", gameDir: "Dossier d'installation", choose: "Parcourir", begin: "Commencer l'installation",
    news: "Actualités", noNews: "Aucune actualité pour le moment.", server: "Serveur", players: "joueurs",
    ready: "Installation prête", notReady: "Installation requise", progress: "Progression de l'installation",
    cancel: "Annuler", addMods: "Ajouter des mods", remove: "Retirer", enable: "Activer", disable: "Désactiver",
    managed: "Géré par AutoModpack", noMods: "Aucun mod installé.", shaders: "Shaders", resourcepacks: "Packs de ressources",
    add: "Ajouter", openFolder: "Ouvrir le dossier", close: "Fermer", createBackup: "Créer une sauvegarde",
    restore: "Restaurer", noBackups: "Aucune sauvegarde.", restoreConfirm: "Restaurer cette sauvegarde ? Les mondes actuels seront remplacés (une sauvegarde de sécurité est créée).",
    exportDiag: "Exporter le diagnostic", copied: "Copié", refresh: "Actualiser le serveur",
    gameRunning: "Minecraft est en cours d'exécution.", fatal: "Une erreur est survenue.", installingShort: "Installation en cours…",
    loading: "Chargement…", profile: "Profil", repair: "Réparer", folder: "Dossier de jeu",
    updateAvailable: "Mise à jour disponible", mandatoryUpdate: "Mise à jour obligatoire", serverEyebrow: "Serveur Minecraft",
    updateTitle: "Mise à jour requise", updateBody: "Une mise à jour obligatoire du launcher doit être installée avant de continuer.",
    updateBlocked: "Impossible de vérifier la mise à jour. Le launcher ne peut pas continuer sans vérification.",
    installUpdate: "Installer la mise à jour", retryUpdate: "Réessayer", updateProgress: "Téléchargement de la mise à jour",
    updatedTo: "Launcher mis à jour vers", openLogs: "Ouvrir les logs", inGame: "En jeu", account: "Compte", mojang: "Statut Mojang", status: "Statut",
    serverDetails: "Détails du serveur", latency: "Latence", playersLabel: "Joueurs", checkedAt: "Vérifié",
    notices: "Notifications", serverOfflineHint: "Minecraft s'ouvrira au menu principal.",
    step_prepare: "Préparation", step_java: "Java", step_minecraft: "Minecraft", step_libraries: "Bibliothèques",
    step_assets: "Ressources", step_neoforge: "NeoForge", step_automodpack: "AutoModpack", step_instance: "Instance",
    step_server: "Serveur", step_verify: "Vérification", pending: "En attente", running: "En cours", done: "Terminé",
    datapacks: "Datapacks", worlds: "Mondes", copyDiag: "Copier le résumé", noWorlds: "Aucun monde pour le moment. Lancez Minecraft une fois pour créer un monde.",
    addDatapack: "Ajouter un datapack", worldMissing: "Choisissez d'abord un monde.",
    warning: "Avertissement", failed: "Échec", onboard_username: "Choisissez un pseudo (3 à 16 caractères).",
    serverHint: "Minecraft s'ouvrira au menu principal. La connexion au serveur reste manuelle.",
    back: "Retour", error_install: "L'installation n'est pas terminée. Corrigez l'erreur puis réessayez.",
    modsHint: "Globez vos fichiers .jar ou utilisez le sélecteur. Les fichiers AutoModpack sont protégés.",
    manageMods: "Gérer les mods", content: "Contenu", library: "Bibliothèque",
  },
  en: {
    play: "Play", install: "Install", installing: "Installing…", launching: "Launching…",
    online: "Online", offline: "Offline", unknown: "Status unknown", maintenance: "Maintenance",
    settings: "Settings", mods: "Mods", backups: "Backups", diagnostics: "Diagnostics",
    welcome: "Welcome to Sky & Beyond", intro: "The sky is only the beginning. Choose a username and folder; the launcher does the rest.",
    username: "Minecraft username", gameDir: "Installation folder", choose: "Browse", begin: "Begin installation",
    news: "News", noNews: "No news right now.", server: "Server", players: "players",
    ready: "Installation ready", notReady: "Installation required", progress: "Installation progress",
    cancel: "Cancel", addMods: "Add mods", remove: "Remove", enable: "Enable", disable: "Disable",
    managed: "Managed by AutoModpack", noMods: "No mods installed.", shaders: "Shaders", resourcepacks: "Resource packs",
    add: "Add", openFolder: "Open folder", close: "Close", createBackup: "Create backup",
    restore: "Restore", noBackups: "No backups yet.", restoreConfirm: "Restore this backup? Current worlds will be replaced (a safety backup is created first).",
    exportDiag: "Export diagnostics", copied: "Copied", refresh: "Refresh server",
    gameRunning: "Minecraft is running.", fatal: "Something went wrong.", installingShort: "Installing…",
    loading: "Loading…", profile: "Profile", repair: "Repair", folder: "Game folder",
    updateAvailable: "Update available", mandatoryUpdate: "Mandatory update", serverEyebrow: "Minecraft server",
    updateTitle: "Update required", updateBody: "A mandatory launcher update must be installed before you can continue.",
    updateBlocked: "The launcher could not verify the update. It cannot continue without verification.",
    installUpdate: "Install update", retryUpdate: "Retry", updateProgress: "Downloading update",
    updatedTo: "Launcher updated to", openLogs: "Open logs", inGame: "In game", account: "Account", mojang: "Mojang status", status: "Status",
    serverDetails: "Server details", latency: "Latency", playersLabel: "Players", checkedAt: "Checked",
    notices: "Notices", serverOfflineHint: "Minecraft opens at the main menu.",
    step_prepare: "Preparation", step_java: "Java", step_minecraft: "Minecraft", step_libraries: "Libraries",
    step_assets: "Assets", step_neoforge: "NeoForge", step_automodpack: "AutoModpack", step_instance: "Instance",
    step_server: "Server", step_verify: "Verification", pending: "Pending", running: "Running", done: "Done",
    datapacks: "Datapacks", worlds: "Worlds", copyDiag: "Copy summary", noWorlds: "No worlds yet. Launch Minecraft once to create a world.",
    addDatapack: "Add datapack", worldMissing: "Choose a world first.",
    warning: "Warning", failed: "Failed", onboard_username: "Choose a username (3–16 characters).",
    serverHint: "Minecraft opens at the main menu. Joining the server stays manual.",
    back: "Back", error_install: "Installation is not complete. Fix the error and try again.",
    modsHint: "Drop .jar files or use the picker. AutoModpack files are protected.",
    manageMods: "Manage mods", content: "Content", library: "Library",
  },
};

export function pickText(value: Localized | undefined, lang: Lang): string {
  if (!value) return "";
  if (typeof value === "string") return value;
  return (lang === "fr" ? value.fr || value.en : value.en || value.fr) || "";
}

export function bytes(value: number): string {
  if (value > 1024 * 1024 * 1024) return `${(value / 1024 / 1024 / 1024).toFixed(1)} GB`;
  if (value > 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
  if (value > 1024) return `${(value / 1024).toFixed(0)} KB`;
  return `${value} B`;
}
