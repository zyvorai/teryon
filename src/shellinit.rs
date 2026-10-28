//! `teryon shell-init`: the few lines a shell needs to tell TERYON where prompts, commands and the
//! working directory are (OSC 133 and OSC 7). Everything works without them, only less smartly.
use std::path::{Path, PathBuf};

/// A `ZDOTDIR` whose only file restores the real one, runs the user's `.zshenv`, and loads the shell
/// integration at the first prompt (after `.zshrc`), so panes get it without editing any startup file.
const ZSH_ENV: &str = r#"# TERYON: undo the temporary ZDOTDIR, run your .zshenv, load the shell integration at the first prompt.
if [[ -n ${TERYON_USER_ZDOTDIR+x} ]]; then
  if [[ -n $TERYON_USER_ZDOTDIR ]]; then ZDOTDIR=$TERYON_USER_ZDOTDIR; else unset ZDOTDIR; fi
  unset TERYON_USER_ZDOTDIR
fi
[[ -r ${ZDOTDIR:-$HOME}/.zshenv ]] && source ${ZDOTDIR:-$HOME}/.zshenv
if [[ -o interactive && -z $__teryon_integrated && "${TERYON_INJECT:-1}" != 0 ]]; then
  __teryon_boot() {
    add-zsh-hook -d precmd __teryon_boot
    unfunction __teryon_boot
    [[ -n $__teryon_integrated ]] && return
    # Use the same build as the daemon: a stale `teryon` earlier on PATH would load an older integration.
    [[ -x ~/.teryon/bin/teryon && -x $TERYON_EXE && "${commands[teryon]:A}" != "${TERYON_EXE:A}" ]] && path=(~/.teryon/bin $path)
    (( $+commands[teryon] )) && eval "$(command teryon shell-init zsh)"
    if [[ -n $__teryon_integrated && ! -e ~/.teryon/notice-seen ]]; then
      print -r -- "teryon: shell integration loaded for this pane (Ctrl-R history, suggestions, Tab, z; ~/.zsh_history imported once). Check it with \`teryon doctor --shell\`; turn it off with [shell] inject = false in the config." >&2
      : > ~/.teryon/notice-seen 2>/dev/null
    fi
  }
  autoload -Uz add-zsh-hook && add-zsh-hook precmd __teryon_boot
fi
"#;

/// Point `~/.teryon/bin/teryon` at the running binary, so zsh panes can put exactly this build first on PATH.
fn link_self() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let link = crate::util::home()?.join(".teryon/bin/teryon");
    if std::fs::read_link(&link).ok().as_deref() == Some(exe.as_path()) {
        return Ok(());
    }
    std::fs::create_dir_all(link.parent().unwrap())?;
    let tmp = link.with_extension("new");
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(&exe, &tmp)?;
    std::fs::rename(&tmp, &link)?;
    Ok(())
}

/// Write the injection directory (under `~/.teryon`) and return it. The daemon points zsh panes'
/// `ZDOTDIR` here and keeps the user's own value in `TERYON_USER_ZDOTDIR`.
pub fn zsh_inject_dir() -> anyhow::Result<PathBuf> {
    let _ = link_self();
    let dir = crate::util::home()?.join(".teryon/zdotdir");
    let file = dir.join(".zshenv");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(ZSH_ENV) {
        std::fs::create_dir_all(&dir)?;
        crate::util::write_private_atomic(&file, ZSH_ENV.as_bytes())?;
    }
    Ok(dir)
}

const ZSH: &str = r#"# TERYON shell integration for zsh (OSC 133 prompt marks, OSC 7 working directory, history, suggestions).
if [[ -z "$__teryon_integrated" ]]; then
  typeset -g __teryon_integrated=1 __teryon_ran= __teryon_cmd= __teryon_t0=
  zmodload zsh/datetime 2>/dev/null
  __teryon_precmd() {
    local s=$?
    [[ -n "$__teryon_ran" ]] && printf '\e]133;D;%s\a' "$s"
    (( $+functions[__teryon_record] )) && __teryon_record "$s"
    __teryon_ran=
    local p=${PWD//\%/%25}; p=${p// /%20}; p=${p//$'\n'/%0A}
    printf '\e]7;file://%s%s\a' "${HOST}" "$p"
    printf '\e]133;A\a'
  }
  __teryon_preexec() { __teryon_ran=1; __teryon_cmd=$1; __teryon_t0=$EPOCHREALTIME; local c=${1//[[:cntrl:]]/ }; printf '\e]133;C;%s\a' "${c[1,500]}"; }
  autoload -Uz add-zsh-hook
  add-zsh-hook precmd __teryon_precmd
  add-zsh-hook preexec __teryon_preexec
fi
# History, autosuggestions, syntax highlighting, fuzzy pickers (Ctrl-R history, Ctrl-T files, Alt-C directories) fuzzy Tab, z/zi directory jump and an opt-in prompt. Set TERYON_PLUS=0 before this line to turn them off.
if [[ -o interactive && "${TERYON_PLUS:-1}" != 0 && -z "$__teryon_plus" ]]; then
  typeset -g __teryon_plus=1 __teryon_hint= __teryon_last_status=0 __teryon_last_ms=0
  typeset -ga __teryon_hls; __teryon_hls=()
  __teryon_record() {
    __teryon_last_status=$1 __teryon_last_ms=0
    [[ -n "$__teryon_cmd" && -n "$__teryon_t0" ]] || return
    local -i ms=$(( (EPOCHREALTIME - __teryon_t0) * 1000 ))
    __teryon_last_ms=$ms
    [[ "${TERYON_HISTORY:-1}" != 0 ]] && command teryon hist add --exit "$1" --ms "$ms" --cwd "$PWD" -- "$__teryon_cmd" >/dev/null 2>&1 &!
    __teryon_cmd= __teryon_t0=
  }
  if [[ "${TERYON_HISTORY:-1}" != 0 ]]; then
    # Write each command to HISTFILE as it runs (a pane that is killed or never exits loses its history otherwise).
    [[ "${TERYON_HISTFLUSH:-1}" != 0 && ! -o inc_append_history && ! -o inc_append_history_time && ! -o share_history ]] && setopt inc_append_history
    # Once: bring ~/.zsh_history into Ctrl-R and the suggestions.
    [[ -e ~/.teryon/zsh-history-imported ]] || command teryon hist import --once >/dev/null 2>&1 &!
    # Copy the history of the hosts named in [shell] sync_hosts (at most every sync_every_hours).
    command teryon hist sync --auto >/dev/null 2>&1 &!
  fi
  __teryon_clear_hint() {
    POSTDISPLAY=
    __teryon_hint=
    local h
    for h in "${__teryon_hls[@]}"; do region_highlight=("${(@)region_highlight:#$h}"); done
    __teryon_hls=()
  }
  __teryon_highlight() {
    [[ -z "$ZSH_HIGHLIGHT_VERSION" && -n "$BUFFER" && "${#BUFFER}" -le 500 && "$BUFFER" != *$'\n'* ]] || return
    local line w; local -a f
    for line in "${(@f)$(command teryon highlight -- "$BUFFER" 2>/dev/null)}"; do
      f=(${=line}); [[ "${#f}" -eq 3 ]] || continue
      if [[ "$f[3]" == '?' ]]; then
        w=${BUFFER:$f[1]:$(( f[2] - f[1] ))}
        if [[ -n "${aliases[$w]+x}${functions[$w]+x}" ]]; then f[3]=fg=green; else f[3]=fg=red; fi
      fi
      __teryon_hls+=("$f[1] $f[2] $f[3]")
    done
    region_highlight+=("${__teryon_hls[@]}")
  }
  __teryon_suggest() {
    [[ -z "$ZSH_AUTOSUGGEST_STRATEGY" && -n "$BUFFER" && "$CURSOR" -eq "${#BUFFER}" && "$BUFFER" != *$'\n'* ]] || return
    local s; s=$(command teryon hist suggest --cwd "$PWD" -- "$BUFFER" 2>/dev/null) || return
    [[ -n "$s" && "$s" == "$BUFFER"?* ]] || return
    __teryon_hint=$s
    POSTDISPLAY=${s#"$BUFFER"}
    local h="${#BUFFER} $(( ${#BUFFER} + ${#POSTDISPLAY} )) fg=8"
    __teryon_hls+=("$h")
    region_highlight+=("$h")
  }
  __teryon_redraw() {
    __teryon_clear_hint
    [[ "${TERYON_HIGHLIGHT:-1}" != 0 ]] && __teryon_highlight
    [[ "${TERYON_SUGGEST:-1}" != 0 ]] && __teryon_suggest
    return 0
  }
  __teryon_accept_all() {
    if [[ -n "$__teryon_hint" && "$CURSOR" -eq "${#BUFFER}" ]]; then
      BUFFER=$__teryon_hint; CURSOR=${#BUFFER}; __teryon_clear_hint
    else
      zle "${1:-.forward-char}"
    fi
  }
  __teryon_forward_char() { __teryon_accept_all .forward-char; }
  __teryon_end_of_line() { __teryon_accept_all .end-of-line; }
  __teryon_forward_word() {
    if [[ -n "$__teryon_hint" && "$CURSOR" -eq "${#BUFFER}" ]]; then
      local rest=${__teryon_hint#"$BUFFER"} lead tail
      lead=${rest%%[![:space:]]*}; tail=${rest#"$lead"}
      BUFFER+=$lead${tail%%[[:space:]]*}; CURSOR=${#BUFFER}
    else
      zle .forward-word
    fi
  }
  __teryon_history_widget() {
    zle -I
    local out rc
    out=$(command teryon pick --history --cwd "$PWD" --query "$BUFFER" 2>/dev/null); rc=$?
    zle reset-prompt
    [[ $rc -eq 0 || $rc -eq 10 ]] || return 0
    BUFFER=$out; CURSOR=${#BUFFER}
    (( rc == 10 )) && zle accept-line
    return 0
  }
  __teryon_file_widget() {
    zle -I
    local out
    out=$(command teryon pick --files --cwd "$PWD" 2>/dev/null) || { zle reset-prompt; return 0; }
    LBUFFER+="${(q)out} "
    zle reset-prompt
  }
  __teryon_cd_widget() {
    zle -I
    local out
    out=$(command teryon pick --dirs --cwd "$PWD" 2>/dev/null) || { zle reset-prompt; return 0; }
    BUFFER="cd -- ${(q)out}"
    zle accept-line
  }
  # Fuzzy Tab: collect what zsh's completion would offer, pick with teryon pick, insert the choice.
  typeset -ga __tc_words; __tc_words=()
  typeset -g __tc_word= __tc_files= __teryon_tab_prev=${${(z)"$(bindkey '^I')"}[2]:-expand-or-complete}
  __teryon_collect_fn() {
    __tc_words=(); __tc_files=; __tc_word=$IPREFIX$PREFIX
    compadd() {
      local a c=; local -a hit
      for a in "$@"; do
        [[ $a == -- ]] && break
        [[ $a == -O || $a == -A || $a == -D ]] && return 1
        [[ $a == -*[fWpP]* ]] && c=1
      done
      [[ -n $c ]] && { __tc_files=1; return 0; }
      builtin compadd -O hit "$@"
      __tc_words+=("${hit[@]}")
      return $(( ${#hit} == 0 ))
    }
    _main_complete
    unfunction compadd
    return 1
  }
  zle -C __teryon_collect complete-word __teryon_collect_fn
  __teryon_tab() {
    local word=${LBUFFER##*[[:space:]]} bad=$'\'"\\$`'
    if [[ -z "${LBUFFER//[[:space:]]/}" || ( -n "$RBUFFER" && "$RBUFFER" != [[:space:]]* ) || "$word" == *[$bad]* \
          || "${TERYON_TAB:-1}" == 0 || -z "$functions[_main_complete]" || -n "$functions[_fzf_tab_complete]" ]]; then
      zle "$__teryon_tab_prev"; return
    fi
    zle __teryon_collect
    local w=$__tc_word dir= base
    [[ "$LBUFFER" == *"$w" ]] || { zle "$__teryon_tab_prev"; return; }
    local -a cands=("${(@u)__tc_words}")
    if [[ -n "$__tc_files" ]]; then
      setopt local_options extended_glob
      dir=${w%"${w##*/}"}; base=${w##*/}
      [[ "$base" == .* ]] && setopt local_options glob_dots
      local -a ds fs flist
      ds=(${~${dir:-./}}*(N-/:t)); fs=(${~${dir:-./}}*(N^-/:t))
      flist=(${^ds}/ "${(@)fs}")
      [[ -n "$dir" ]] && flist=("${(@)flist/#/$dir}")
      cands+=("${flist[@]}")
    fi
    local -a pre=(${(M)cands:#${(b)w}*})
    local pick
    if (( ${#cands} == 0 )); then
      zle "$__teryon_tab_prev"; return
    elif (( ${#pre} == 1 )); then
      pick=$pre[1]
    else
      pick=$(print -rl -- "${cands[@]}" | command teryon pick --query "$w" 2>/dev/null) || { zle reset-prompt; return 0; }
      zle reset-prompt
    fi
    local tail=' '
    [[ "$pick" == *[/=:] || -n "$RBUFFER" ]] && tail=
    if [[ -n "$dir" && "$pick" == "$dir"* ]]; then
      LBUFFER="${LBUFFER%"$w"}${dir}${(q)${pick#"$dir"}}$tail"
    else
      LBUFFER="${LBUFFER%"$w"}${(q)pick}$tail"
    fi
  }
  zle -N __teryon_tab
  # Jump: every directory you enter is remembered; z picks the best match, zi lets you choose.
  if [[ "${TERYON_JUMP:-1}" != 0 ]]; then
    __teryon_chpwd() { command teryon jump add "$PWD" >/dev/null 2>&1 &! }
    add-zsh-hook chpwd __teryon_chpwd
    __teryon_chpwd
    if (( ! $+functions[z] && ! $+aliases[z] )); then
      z() {
        if (( $# == 0 )); then builtin cd ~; return; fi
        if (( $# == 1 )) && [[ -d "$1" || "$1" == - ]]; then builtin cd -- "$1"; return; fi
        local d; d=$(command teryon jump query --cwd "$PWD" -- "$@") || { print -u2 "z: no directory matches: $*"; return 1; }
        builtin cd -- "$d"
      }
    fi
    if (( ! $+functions[zi] && ! $+aliases[zi] )); then
      zi() {
        local d; d=$(command teryon jump list --cwd "$PWD" -- "$@" | command teryon pick --query "$*") || return
        [[ -n "$d" ]] && builtin cd -- "$d"
      }
    fi
  fi
  # Prompt (opt-in): set TERYON_PROMPT=1 before this line for a two-line prompt with git state.
  if [[ "${TERYON_PROMPT:-0}" == 1 ]]; then
    setopt prompt_subst
    typeset -g __teryon_prompt=
    __teryon_prompt_hook() {
      __teryon_prompt=$(command teryon prompt --status "$__teryon_last_status" --ms "$__teryon_last_ms" --cwd "$PWD" 2>/dev/null)
    }
    add-zsh-hook precmd __teryon_prompt_hook
    PROMPT='${__teryon_prompt}'
  fi
  # `teryon-features` shows what is on, and what is yielding to a plugin you already load.
  teryon-features() {
    local name var note state def
    for name var def in History HISTORY 1 Suggestions SUGGEST 1 Highlighting HIGHLIGHT 1 Pickers PICKERS 1 "Fuzzy Tab" TAB 1 "z and zi" JUMP 1 Prompt PROMPT 0; do
      note=
      state=on
      if [[ "${(P)${:-TERYON_$var}:-$def}" == 0 || ( "$var" == PROMPT && "${TERYON_PROMPT:-0}" != 1 ) ]]; then state=off; fi
      [[ $var == HIGHLIGHT && -n "$ZSH_HIGHLIGHT_VERSION" ]] && note="yielding to zsh-syntax-highlighting"
      [[ $var == SUGGEST && -n "$ZSH_AUTOSUGGEST_STRATEGY" ]] && note="yielding to zsh-autosuggestions"
      [[ $var == TAB && ( "$__teryon_tab_prev" == *fzf-tab* || -n "$functions[_fzf_tab_complete]" ) ]] && note="yielding to fzf-tab"
      [[ $var == TAB && -z "$functions[_main_complete]" ]] && note="needs compinit"
      printf '%-13s %-4s %s\n' "$name" "$state" "$note"
    done
  }
  autoload -Uz add-zle-hook-widget
  add-zle-hook-widget line-pre-redraw __teryon_redraw
  add-zle-hook-widget line-finish __teryon_clear_hint
  zle -N __teryon_forward_char; zle -N __teryon_end_of_line; zle -N __teryon_forward_word
  zle -N __teryon_history_widget; zle -N __teryon_file_widget; zle -N __teryon_cd_widget
  [[ "${TERYON_PICKERS:-1}" != 0 ]] && bindkey '^R' __teryon_history_widget '^T' __teryon_file_widget '^[c' __teryon_cd_widget
  [[ "$__teryon_tab_prev" == *fzf-tab* ]] || bindkey '^I' __teryon_tab
  bindkey '^[[C' __teryon_forward_char '^[OC' __teryon_forward_char '^E' __teryon_end_of_line '^[f' __teryon_forward_word
fi
"#;

const BASH: &str = r#"# TERYON shell integration for bash (OSC 133 prompt marks, OSC 7 working directory, history, pickers, z).
if [ -z "$__teryon_integrated" ]; then
  __teryon_integrated=1
  __teryon_ran=
  __teryon_t0=
  __teryon_hnum=
  __teryon_last_pwd=
  __teryon_precmd() {
    local s=$?
    if [ -n "$__teryon_ran" ]; then printf '\e]133;D;%s\a' "$s"; fi
    declare -F __teryon_record >/dev/null && __teryon_record "$s"
    __teryon_ran=
    local p=${PWD//\%/%25}; p=${p// /%20}
    printf '\e]7;file://%s%s\a' "${HOSTNAME}" "$p"
    printf '\e]133;A\a'
    return "$s"
  }
  __teryon_preexec() {
    [ -n "$__teryon_ran" ] && return
    case "$BASH_COMMAND" in __teryon_*|*PROMPT_COMMAND*) return ;; esac
    __teryon_ran=1
    __teryon_t0=${EPOCHREALTIME/[.,]/}
    local c=${BASH_COMMAND//[[:cntrl:]]/ }
    printf '\e]133;C;%s\a' "${c:0:500}"
  }
  PROMPT_COMMAND="__teryon_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
  trap '__teryon_preexec' DEBUG
fi
# History, fuzzy pickers (Ctrl-R history, Ctrl-T files, Alt-C directories) and z/zi directory jump.
# Needs bash 4+ for the pickers. Set TERYON_PLUS=0 before this line to turn them off. There is no ghost
# text, highlighting or fuzzy Tab in bash; Tab stays bash-completion's.
if [[ $- == *i* && "${TERYON_PLUS:-1}" != 0 && -z "$__teryon_plus" ]]; then
  __teryon_plus=1
  __teryon_hist_number() {
    local h; h=$(HISTTIMEFORMAT= builtin history 1 2>/dev/null) || return 1
    [[ $h =~ ^[[:space:]]*([0-9]+)[[:space:]]+(.*)$ ]] || return 1
    __teryon_hn=${BASH_REMATCH[1]}; __teryon_hc=${BASH_REMATCH[2]}
  }
  __teryon_hist_number && __teryon_hnum=$__teryon_hn
  __teryon_record() {
    local s=$1
    if [ "${TERYON_JUMP:-1}" != 0 ] && [ "$PWD" != "$__teryon_last_pwd" ]; then
      __teryon_last_pwd=$PWD
      command teryon jump add "$PWD" >/dev/null 2>&1 & disown 2>/dev/null
    fi
    [ "${TERYON_HISTORY:-1}" = 0 ] && return 0
    # bash 3.2 (macOS /bin/bash) never writes anything for `history -a`; it appends the line itself below.
    (( BASH_VERSINFO[0] >= 4 )) && [ "${TERYON_HISTFLUSH:-1}" != 0 ] && builtin history -a 2>/dev/null
    __teryon_hist_number || return 0
    # A command hidden from bash's history (leading space, ignoredups) has no new number: skip it.
    [ "$__teryon_hn" = "$__teryon_hnum" ] && return 0
    __teryon_hnum=$__teryon_hn
    (( BASH_VERSINFO[0] < 4 )) && [ "${TERYON_HISTFLUSH:-1}" != 0 ] && printf '%s\n' "$__teryon_hc" >> "${HISTFILE:-$HOME/.bash_history}" 2>/dev/null
    local ms=0 now=${EPOCHREALTIME/[.,]/}
    [ -n "$__teryon_t0" ] && [ -n "$now" ] && ms=$(( (now - __teryon_t0) / 1000 ))
    __teryon_t0=
    command teryon hist add --exit "$s" --ms "$ms" --cwd "$PWD" -- "$__teryon_hc" >/dev/null 2>&1 & disown 2>/dev/null
    return 0
  }
  if [ "${TERYON_HISTORY:-1}" != 0 ]; then
    # Append each command to HISTFILE as it runs, so a pane that is killed keeps its history.
    # (bash 3.2 keeps its default of rewriting HISTFILE at exit: with histappend the lines appended above would repeat.)
    (( BASH_VERSINFO[0] >= 4 )) && [ "${TERYON_HISTFLUSH:-1}" != 0 ] && shopt -s histappend
    [ -e ~/.teryon/history-imported-.bash_history ] || [ ! -e ~/.bash_history ] || { command teryon hist import --once ~/.bash_history >/dev/null 2>&1 & disown 2>/dev/null; }
    command teryon hist sync --auto >/dev/null 2>&1 & disown 2>/dev/null
  fi
  if (( BASH_VERSINFO[0] >= 4 )) && [ "${TERYON_PICKERS:-1}" != 0 ]; then
    __teryon_history_widget() {
      local out rc
      out=$(command teryon pick --history --cwd "$PWD" --query "$READLINE_LINE" 2>/dev/null); rc=$?
      [ $rc -eq 0 ] || [ $rc -eq 10 ] || return 0
      READLINE_LINE=$out; READLINE_POINT=${#out}
    }
    __teryon_file_widget() {
      local out q
      out=$(command teryon pick --files --cwd "$PWD" 2>/dev/null) || return 0
      printf -v q '%q ' "$out"
      READLINE_LINE="${READLINE_LINE:0:READLINE_POINT}${q}${READLINE_LINE:READLINE_POINT}"
      READLINE_POINT=$(( READLINE_POINT + ${#q} ))
    }
    __teryon_cd_widget() {
      local out q
      out=$(command teryon pick --dirs --cwd "$PWD" 2>/dev/null) || return 0
      printf -v q '%q' "$out"
      READLINE_LINE="cd -- $q"; READLINE_POINT=${#READLINE_LINE}
    }
    bind -x '"\C-r": __teryon_history_widget'
    bind -x '"\C-t": __teryon_file_widget'
    bind -x '"\ec": __teryon_cd_widget'
  fi
  if [ "${TERYON_JUMP:-1}" != 0 ]; then
    if ! type z >/dev/null 2>&1; then
      z() {
        if [ $# -eq 0 ]; then builtin cd ~; return; fi
        if [ $# -eq 1 ] && { [ -d "$1" ] || [ "$1" = - ]; }; then builtin cd -- "$1"; return; fi
        local d; d=$(command teryon jump query --cwd "$PWD" -- "$@") || { echo "z: no directory matches: $*" >&2; return 1; }
        builtin cd -- "$d"
      }
    fi
    if ! type zi >/dev/null 2>&1; then
      zi() {
        local d; d=$(command teryon jump list --cwd "$PWD" -- "$@" | command teryon pick --query "$*") || return
        [ -n "$d" ] && builtin cd -- "$d"
      }
    fi
  fi
fi
"#;

const FISH: &str = r#"# TERYON shell integration for fish (OSC 133 prompt marks, OSC 7 working directory, history, pickers, z).
if not set -q __teryon_integrated
  set -g __teryon_integrated 1
  function __teryon_prompt --on-event fish_prompt
    set -l s $status
    if set -q __teryon_ran
      printf '\e]133;D;%s\a' $s
      set -e __teryon_ran
    end
    printf '\e]7;file://%s%s\a' (hostname) (string replace -a '%' '%25' -- $PWD | string replace -a ' ' '%20')
    printf '\e]133;A\a'
  end
  function __teryon_preexec --on-event fish_preexec
    set -g __teryon_ran 1
    printf '\e]133;C;%s\a' (string replace -ra '[[:cntrl:]]' ' ' -- $argv[1] | string sub -l 500)
  end
end
# History, fuzzy pickers (Ctrl-R history, Ctrl-T files, Alt-C directories) and z/zi directory jump.
# Set TERYON_PLUS=0 before this line to turn them off. fish keeps its own history file and Tab.
if status is-interactive; and test "$TERYON_PLUS" != 0; and not set -q __teryon_plus
  set -g __teryon_plus 1
  if test "$TERYON_HISTORY" != 0
    function __teryon_record --on-event fish_postexec
      set -l s $status
      set -l ms 0
      set -q CMD_DURATION; and set ms $CMD_DURATION
      command teryon hist add --exit $s --ms $ms --cwd $PWD -- $argv[1] >/dev/null 2>&1 &
      disown 2>/dev/null
    end
    command teryon hist sync --auto >/dev/null 2>&1 &
    disown 2>/dev/null
  end
  if test "$TERYON_JUMP" != 0
    function __teryon_chpwd --on-variable PWD
      command teryon jump add $PWD >/dev/null 2>&1 &
      disown 2>/dev/null
    end
    if not functions -q z
      function z
        if test (count $argv) -eq 0
          builtin cd ~
          return
        end
        if test (count $argv) -eq 1; and begin; test -d "$argv[1]"; or test "$argv[1]" = -; end
          builtin cd -- $argv[1]
          return
        end
        set -l d (command teryon jump query --cwd $PWD -- $argv)
        or begin; echo "z: no directory matches: $argv" >&2; return 1; end
        builtin cd -- $d
      end
    end
    if not functions -q zi
      function zi
        set -l d (command teryon jump list --cwd $PWD -- $argv | command teryon pick --query "$argv")
        and test -n "$d"; and builtin cd -- $d
      end
    end
  end
  if test "$TERYON_PICKERS" != 0
    function __teryon_history_widget
      set -l out (command teryon pick --history --cwd $PWD --query (commandline) 2>/dev/null)
      set -l rc $status
      if test $rc -eq 0 -o $rc -eq 10
        commandline -r -- $out
        commandline -f repaint
        test $rc -eq 10; and commandline -f execute
      else
        commandline -f repaint
      end
    end
    function __teryon_file_widget
      set -l out (command teryon pick --files --cwd $PWD 2>/dev/null)
      and commandline -i -- (string escape -- $out)" "
      commandline -f repaint
    end
    function __teryon_cd_widget
      set -l out (command teryon pick --dirs --cwd $PWD 2>/dev/null)
      and begin
        commandline -r -- "cd -- "(string escape -- $out)
        commandline -f execute
      end
      commandline -f repaint
    end
    bind \cr __teryon_history_widget
    bind \ct __teryon_file_widget
    bind \ec __teryon_cd_widget
  end
end
"#;

/// Lines that make the `[shell]` config the default for each switch. A variable already set in the
/// environment wins (`${VAR:=...}` only fills in an unset one).
pub fn preamble(cfg: &crate::config::ShellConfig) -> String {
    let flag = |b: bool| u8::from(b);
    [
        ("TERYON_PLUS", cfg.enabled),
        ("TERYON_HISTORY", cfg.history),
        ("TERYON_SUGGEST", cfg.suggest),
        ("TERYON_HIGHLIGHT", cfg.highlight),
        ("TERYON_PICKERS", cfg.pickers),
        ("TERYON_TAB", cfg.tab),
        ("TERYON_JUMP", cfg.jump),
        ("TERYON_PROMPT", cfg.prompt),
    ]
    .iter()
    .map(|(name, on)| format!(": \"${{{name}:={}}}\"\n", flag(*on)))
    .collect()
}

pub const SHELLS: [&str; 3] = ["zsh", "bash", "fish"];

/// The integration script for `shell` (`zsh`, `bash` or `fish`).
pub fn script(shell: &str) -> Option<&'static str> {
    match shell {
        "zsh" => Some(ZSH),
        "bash" => Some(BASH),
        "fish" => Some(FISH),
        _ => None,
    }
}
/// The shell named by `$SHELL` (just the file name), if it is one we support.
pub fn detect(shell_env: &str) -> Option<&'static str> {
    let name = Path::new(shell_env).file_name()?.to_str()?;
    SHELLS.into_iter().find(|s| *s == name)
}
/// Start-up file where the loader line goes.
pub fn rc_file(shell: &str, home: &Path) -> PathBuf {
    match shell {
        "zsh" => home.join(".zshrc"),
        "fish" => home.join(".config/fish/conf.d/teryon.fish"),
        _ if cfg!(target_os = "macos") => home.join(".bash_profile"),
        _ => home.join(".bashrc"),
    }
}
const START: &str = "# >>> teryon shell integration >>>";
const END: &str = "# <<< teryon shell integration <<<";
/// The marked block that loads the integration (guarded, so a machine without teryon is unaffected).
pub fn block(shell: &str) -> String {
    let load = if shell == "fish" {
        "command -q teryon; and teryon shell-init fish | source".to_string()
    } else {
        format!("command -v teryon >/dev/null 2>&1 && eval \"$(teryon shell-init {shell})\"")
    };
    format!("{START}\n{load}\n{END}\n")
}
/// `existing` with the block added or refreshed; the flag says whether anything changed.
pub fn ensure_block(existing: &str, block: &str) -> (String, bool) {
    if let (Some(a), Some(b)) = (existing.find(START), existing.find(END)) {
        if a < b {
            let end = b + END.len();
            let end = if existing[end..].starts_with('\n') {
                end + 1
            } else {
                end
            };
            let updated = format!("{}{}{}", &existing[..a], block, &existing[end..]);
            let changed = updated != existing;
            return (updated, changed);
        }
    }
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(block);
    (out, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    #[test]
    fn every_script_emits_the_marks_teryon_reads() {
        for shell in SHELLS {
            let s = script(shell).unwrap();
            for needle in ["133;A", "133;C", "133;D", "]7;file://"] {
                assert!(s.contains(needle), "{shell} lacks {needle}");
            }
        }
        assert!(script("tcsh").is_none());
    }
    #[test]
    fn scripts_are_syntactically_valid_for_the_shells_that_are_installed() {
        let dir = tempfile::tempdir().unwrap();
        for shell in SHELLS {
            let path = dir.path().join(format!("init.{shell}"));
            std::fs::write(&path, script(shell).unwrap()).unwrap();
            let Ok(status) = Command::new(shell)
                .arg("-n")
                .arg(&path)
                .stdin(Stdio::null())
                .status()
            else {
                continue;
            };
            assert!(status.success(), "{shell} rejected its integration script");
        }
    }
    #[test]
    fn zsh_script_records_history_and_binds_the_suggestion_keys() {
        let z = script("zsh").unwrap();
        for needle in [
            "hist add",
            "hist suggest",
            "line-pre-redraw",
            "teryon highlight",
            "pick --history",
            "pick --files",
            "pick --dirs",
            "TERYON_PLUS",
            "__teryon_forward_word",
        ] {
            assert!(z.contains(needle), "zsh script lacks {needle}");
        }
    }
    #[test]
    fn the_preamble_turns_config_into_defaults_that_the_environment_can_override() {
        let cfg = crate::config::ShellConfig {
            prompt: true,
            tab: false,
            ..Default::default()
        };
        let p = preamble(&cfg);
        assert!(
            p.contains(": \"${TERYON_PROMPT:=1}\"\n") && p.contains(": \"${TERYON_TAB:=0}\"\n")
        );
        assert!(p.contains(": \"${TERYON_PLUS:=1}\"\n"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("init.zsh");
        std::fs::write(&path, format!("{p}{}", script("zsh").unwrap())).unwrap();
        let Ok(status) = Command::new("zsh").arg("-n").arg(&path).status() else {
            return;
        };
        assert!(status.success());
        let out = Command::new("zsh")
            .args([
                "-fc",
                "TERYON_PROMPT=0; source \"$1\"; print -r -- \"$TERYON_PROMPT $TERYON_TAB\"",
            ])
            .arg("x")
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "0 0");
    }
    #[test]
    fn detection_and_rc_files() {
        assert_eq!(detect("/bin/zsh"), Some("zsh"));
        assert_eq!(detect("/opt/homebrew/bin/fish"), Some("fish"));
        assert_eq!(detect("/bin/tcsh"), None);
        let home = Path::new("/h");
        assert_eq!(rc_file("zsh", home), PathBuf::from("/h/.zshrc"));
        assert!(rc_file("fish", home).ends_with("conf.d/teryon.fish"));
    }
    #[test]
    fn the_rc_block_is_added_once_refreshed_in_place_and_leaves_other_text_alone() {
        let b = block("zsh");
        assert!(b.contains("command -v teryon") && b.contains("shell-init zsh"));
        let (first, changed) = ensure_block("export A=1", &b);
        assert!(changed && first.starts_with("export A=1\n\n# >>>"));
        let (again, changed) = ensure_block(&first, &b);
        assert!(!changed && again == first);
        let old = first.replace("shell-init zsh", "shell-init zsh --old");
        let (fixed, changed) = ensure_block(&format!("{old}tail\n"), &b);
        assert!(changed && fixed.ends_with("tail\n") && !fixed.contains("--old"));
        assert_eq!(fixed.matches(START).count(), 1);
        let (empty, _) = ensure_block("", &b);
        assert_eq!(empty, b);
    }
}
