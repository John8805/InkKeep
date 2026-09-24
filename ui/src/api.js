// Tauri command 的薄包裝。錯誤原樣往上拋。
import { invoke } from "@tauri-apps/api/core";

export const vaultState = () => invoke("vault_state");
export const unlock = (password) => invoke("unlock", { password });
export const unlockSecrets = (password) => invoke("unlock_secrets", { password });
export const revealVaultKey = () => invoke("reveal_vault_key");
export const forgetVaultKey = () => invoke("forget_vault_key");
export const createVault = (password, withSamples) =>
  invoke("create_vault", { password, withSamples });
export const checkVaultPath = (path) => invoke("check_vault_path", { path });
export const setVaultPath = (path) => invoke("set_vault_path", { path });
export const lock = () => invoke("lock");
export const search = (query, tags, limit, kind, workspace) =>
  invoke("search_items", { query, tags, limit, kind, workspace });
export const itemGet = (id) => invoke("item_get", { id });
export const revealPassword = (id) => invoke("reveal_password", { id });
export const prepareInsert = (id) => invoke("prepare_insert", { id });
export const preview = (id, inputs) => invoke("preview", { id, inputs });
export const insert = (id, field, inputs) => invoke("insert", { id, field, inputs });
export const openBookmark = (id) => invoke("open_bookmark", { id });
export const copyOnly = (id, field, inputs) => invoke("copy_only", { id, field, inputs });
export const hideWindow = () => invoke("hide_window");
export const itemUpsert = (input) => invoke("item_upsert", { input });
export const itemDelete = (id) => invoke("item_delete", { id });
export const templateValidate = (body) => invoke("template_validate", { body });
export const settingsGet = () => invoke("settings_get");
export const settingsSet = (settings) => invoke("settings_set", { settings });
export const changePassword = (old, neu) => invoke("change_password", { old, new: neu });
export const generatePassword = (opts) => invoke("generate_password", { opts });
export const entropyBits = (opts) => invoke("entropy_bits", { opts });
export const importPreview = (source, path, workspace) =>
  invoke("import_preview", { source, path, workspace });
export const importBookmarks = (source, path, workspace) =>
  invoke("import_bookmarks", { source, path, workspace });
export const tagList = () => invoke("tag_list");
export const importSources = () => invoke("import_sources");
export const passwordImportPreview = (path, workspace) =>
  invoke("password_import_preview", { path, workspace });
export const passwordImport = (path, workspace) => invoke("password_import", { path, workspace });
export const workspaceList = () => invoke("workspace_list");
export const workspaceAdd = (name) => invoke("workspace_add", { name });
export const workspaceRename = (id, name) => invoke("workspace_rename", { id, name });
export const workspaceDelete = (id) => invoke("workspace_delete", { id });
export const deleteImportFile = (path) => invoke("delete_import_file", { path });

// describeError：依語言包把錯誤轉成文字
export { describeError } from "./i18n.svelte.js";
