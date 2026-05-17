-- =============================================================================
-- kyro.lua — Script mpv.net pour Kyro
-- =============================================================================
--
-- Objectif :
--   Pousser périodiquement la position de lecture courante vers le Kyro_Backend
--   via un named pipe Windows (\\.\pipe\kyro-<session_id>) afin que Kyro puisse
--   mettre à jour Watch_Progress en temps réel et garantir une reprise exacte
--   au prochain lancement (Requirements 9.2, 9.3, 9.4).
--
-- Activation :
--   Le Kyro_Player_Bridge passe à mpv.net l'option
--       --script-opts=kyro-pipe=kyro-<session_id>
--   Si cette option est absente, le script se termine silencieusement et n'a
--   aucun effet sur le fonctionnement de mpv.net (mode défensif).
--
-- Contrat NDJSON :
--   Une ligne JSON par message, encodée en UTF-8, terminée par '\n'.
--   Schéma :
--     {"type": <string>, "position": <float>, "duration": <float>, "session": <string>}
--   - position et duration sont en secondes (3 décimales)
--   - session correspond à <session_id> extrait du nom du pipe
--
-- Types de messages émis :
--   - "progress" : envoyé toutes les 5 s pendant la lecture (Requirement 9.2)
--   - "paused"   : envoyé sur l'événement mpv `pause`
--   - "resumed"  : envoyé sur l'événement mpv `unpause`
--   - "closed"   : envoyé sur l'événement mpv `shutdown` avec la dernière
--                  position connue, juste avant la fermeture du pipe
--                  (Requirement 9.3)
--
-- Robustesse :
--   - Si l'option `kyro-pipe` est manquante : sortie silencieuse.
--   - Si l'ouverture du pipe échoue : on retente quelques fois avec un petit
--     délai (le serveur Rust peut prendre un instant à matérialiser le
--     pipe), puis on abandonne silencieusement après échec final.
--   - Aucun popup d'erreur n'est affiché dans mpv.net.
--   - Les diagnostics sont écrits via `mp.msg.info` / `mp.msg.warn` pour
--     apparaître dans le log mpv.net (`%APPDATA%\mpv.net\log.txt`).
-- =============================================================================

local mp = require 'mp'
local msg = require 'mp.msg'

-- Récupère l'option `kyro-pipe` depuis script-opts. mpv.net expose
-- généralement la map directement via `options/script-opts` ; certains
-- builds la rendent uniquement via la fonction utilitaire `mp.options`.
-- On tente d'abord la voie native, puis on retombe sur un parse manuel
-- de la string brute si nécessaire.
local function read_pipe_option()
  local opts = mp.get_property_native("options/script-opts")
  if type(opts) == "table" and opts["kyro-pipe"] then
    return opts["kyro-pipe"]
  end
  -- Fallback : la string brute peut ressembler à
  --   "kyro-pipe=kyro-<uuid>,autre=valeur"
  local raw = mp.get_property("options/script-opts", "")
  if raw and raw ~= "" then
    for entry in raw:gmatch("[^,]+") do
      local k, v = entry:match("^([^=]+)=(.*)$")
      if k and v and k:gsub("^%s+", ""):gsub("%s+$", "") == "kyro-pipe" then
        return v:gsub("^%s+", ""):gsub("%s+$", "")
      end
    end
  end
  return nil
end

local pipe_name = read_pipe_option()
if not pipe_name or pipe_name == "" then
  msg.info("kyro.lua: option kyro-pipe absente — script inactif")
  return
end

local session = pipe_name:match("kyro%-(.+)$") or ""
local full_pipe = "\\\\.\\pipe\\" .. pipe_name

-- Tentative d'ouverture avec retry borné. Sous Windows, mpv.net démarre
-- le script dès le boot du processus ; le serveur Rust côté Kyro a
-- normalement déjà créé le pipe (le bridge crée le NamedPipeServer
-- AVANT de spawn mpv.net), mais on garde un petit retry défensif au cas
-- où la création serait légèrement plus lente que le boot mpv.
local f = nil
local attempts = 0
local max_attempts = 10
while not f and attempts < max_attempts do
  f = io.open(full_pipe, "wb")
  if not f then
    attempts = attempts + 1
    -- Petit busy-wait ~50 ms (mpv.net Lua n'a pas de sleep natif ;
    -- on utilise os.clock comme spin court — acceptable car seulement
    -- au démarrage et borné par max_attempts).
    local target = os.clock() + 0.05
    while os.clock() < target do end
  end
end

if not f then
  msg.warn(string.format(
    "kyro.lua: impossible d'ouvrir le pipe %s après %d tentatives",
    full_pipe, max_attempts))
  return
end

msg.info(string.format("kyro.lua: pipe %s ouvert (session=%s)", full_pipe, session))

-- Cache local des dernières valeurs valides observées. On le remet à
-- jour à chaque tick de la `time-pos` property et à chaque pause/resume.
-- Indispensable au moment du `shutdown` : à cet instant mpv a déjà
-- libéré son état et `mp.get_property_number("time-pos")` retourne nil
-- (et notre fallback retombait sur 0, ce qui écrasait la progression
-- accumulée par un `closed` à 0 — bug observé le 17/05).
local last_pos = 0
local last_dur = 0
-- Path du fichier courant au moment du dernier `path` observé. On
-- l'utilise pour **embarquer le chemin dans le message `completed`**
-- afin que le backend puisse résoudre la cible de complétion sans
-- ambiguïté, même si un `file-changed` arrive en course juste après
-- (cas typique : autoplay mpv → end-file → path-change quasi
-- simultanés).
local last_path = ""

local function refresh_cache()
  local pos = mp.get_property_number("time-pos")
  local dur = mp.get_property_number("duration")
  if pos and pos >= 0 then last_pos = pos end
  if dur and dur > 0 then last_dur = dur end
end

local function send(t, use_cache)
  if not use_cache then
    refresh_cache()
  end
  local line = string.format(
    '{"type":"%s","position":%.3f,"duration":%.3f,"session":"%s"}\n',
    t, last_pos, last_dur, session)
  local ok, err = pcall(function()
    f:write(line)
    f:flush()
  end)
  if not ok then
    msg.warn("kyro.lua: échec d'écriture sur le pipe: " .. tostring(err))
  end
end

-- Observe `time-pos` en continu : à chaque mise à jour mpv, le cache
-- `last_pos` est rafraîchi. Cela garantit qu'au moment du `shutdown`
-- on dispose toujours d'une valeur récente même si mpv a libéré la
-- property entre-temps.
mp.observe_property("time-pos", "number", function(_, value)
  if value and value >= 0 then last_pos = value end
end)
mp.observe_property("duration", "number", function(_, value)
  if value and value > 0 then last_dur = value end
end)

-- Échappe une chaîne Lua pour la sérialisation JSON minimale (string
-- value uniquement) : `\` → `\\`, `"` → `\"`. Suffisant pour les
-- chemins Windows (`C:\Users\...`) et les noms de fichiers
-- standards. Les caractères de contrôle <0x20 sont théoriquement
-- absents d'un chemin de fichier valide, on les laisse passer tels
-- quels (l'overhead d'une fonction d'échappement complète n'est pas
-- nécessaire ici).
local function string_to_json(s)
  s = s:gsub('\\', '\\\\')
  s = s:gsub('"', '\\"')
  return '"' .. s .. '"'
end

-- Observe `path` (chemin du fichier en cours de lecture mpv) : un
-- changement signale un autoplay / playlist-next, et le backend doit
-- alors basculer la cible IPC sur le nouvel épisode (sinon les
-- progressions du nouveau fichier sont écrites sur l'ancien). On
-- réinitialise aussi le cache local pour ne pas mélanger les
-- positions de l'ancien et du nouveau fichier.
mp.observe_property("path", "string", function(_, new_path)
  if not new_path or new_path == "" then return end
  -- Important : on capture le path AVANT le reset du cache local,
  -- pour que le prochain `end-file` puisse continuer à référencer
  -- l'ancien path si la séquence est inversée. Mais comme `end-file`
  -- mpv est garanti d'arriver AVANT `path` change quand le fichier
  -- se finit naturellement (eof → end-file → next file → path), on
  -- met simplement à jour `last_path` ici.
  last_path = new_path
  last_pos = 0
  last_dur = 0
  local line = string.format(
    '{"type":"file-changed","path":%s,"session":"%s"}\n',
    string_to_json(new_path), session)
  local ok, err = pcall(function()
    f:write(line)
    f:flush()
  end)
  if not ok then
    msg.warn("kyro.lua: échec d'écriture file-changed: " .. tostring(err))
  else
    msg.info("kyro.lua: path changé → " .. new_path)
  end
end)

mp.add_periodic_timer(5, function() send("progress", false) end)  -- 9.2
mp.register_event("pause",    function() send("paused",  false) end)
mp.register_event("unpause",  function() send("resumed", false) end)

-- end-file (eof) : mpv signale la fin naturelle du fichier. À ce
-- moment `time-pos` est encore lisible (mpv n'a pas encore commencé
-- le shutdown), donc on rafraîchit le cache puis on envoie un message
-- `completed` avec la durée totale en plus de la position. Ce message
-- précède le `shutdown` de quelques millisecondes et permet au backend
-- de forcer le statut `vu` (cf. PipeMessage::Completed côté Rust).
mp.register_event("end-file", function(event)
  if event and event.reason == "eof" then
    refresh_cache()
    -- On embarque `last_path` dans le payload : c'est le chemin du
    -- fichier qui vient de se terminer, capturé avant qu'un éventuel
    -- `file-changed` ne le mette à jour. Permet au backend de
    -- résoudre la cible de complétion sans ambiguïté même en cas de
    -- course autoplay → file-changed → completed inversée.
    local line = string.format(
      '{"type":"completed","position":%.3f,"duration":%.3f,"session":"%s","path":%s}\n',
      last_pos, last_dur, session, string_to_json(last_path))
    local ok, err = pcall(function()
      f:write(line)
      f:flush()
    end)
    if not ok then
      msg.warn("kyro.lua: échec d'écriture completed: " .. tostring(err))
    else
      msg.info(string.format(
        "kyro.lua: end-of-file (pos=%.1f dur=%.1f path=%s) — completed envoyé",
        last_pos, last_dur, last_path))
    end
  end
end)

mp.register_event("shutdown", function()                              -- 9.3
  -- À l'arrêt, on n'interroge PLUS mpv (state already torn down) :
  -- on envoie le cache avec `use_cache=true` pour que `closed` porte
  -- la dernière position observée pendant la lecture.
  send("closed", true)
  if f then pcall(function() f:close() end) end
end)
