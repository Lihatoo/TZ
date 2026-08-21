use std::io;

use crate::cli::{CompletionCommand, CompletionShell};

pub fn run(command: CompletionCommand) -> Result<(), io::Error> {
    match command {
        CompletionCommand::Generate { shell } => {
            let script = match shell {
                CompletionShell::Bash => bash_script(),
                CompletionShell::Zsh => zsh_script(),
                CompletionShell::Fish => fish_script(),
            };
            print!("{script}");
            Ok(())
        }
    }
}

fn bash_script() -> &'static str {
    r#"_tz_complete() {
  local cur prev command subcommand
  cur="${COMP_WORDS[COMP_CWORD]}"
  prev="${COMP_WORDS[COMP_CWORD-1]}"
  command="${COMP_WORDS[1]}"
  if (( COMP_CWORD == 1 )); then
    COMPREPLY=( $(compgen -W 'init status st start on stop off end restart r list select node tun proxy setting set profile p core c config cfg completion comp -l --list' -- "$cur") )
    return
  fi
  case "$command" in
    profile|p)
      if (( COMP_CWORD == 2 )); then
        COMPREPLY=( $(compgen -W 'add a list l info i use u update up remove rm' -- "$cur") )
      elif [[ "$prev" == "--family" ]]; then
        COMPREPLY=( $(compgen -W 'clash sing-box' -- "$cur") )
      elif [[ "$cur" == -* ]]; then
        COMPREPLY=( $(compgen -W '--family --all' -- "$cur") )
      fi
      ;;
    core|c)
      if (( COMP_CWORD == 2 )); then
        COMPREPLY=( $(compgen -W 'add a list l info i use u remove rm' -- "$cur") )
      fi
      ;;
    setting|set)
      if (( COMP_CWORD == 2 )); then
        COMPREPLY=( $(compgen -W 'list get set reset' -- "$cur") )
      fi
      ;;
    config|cfg)
      if (( COMP_CWORD == 2 )); then
        COMPREPLY=( $(compgen -W 'build check show' -- "$cur") )
      fi
      ;;
    node)
      if (( COMP_CWORD == 2 )); then
        COMPREPLY=( $(compgen -W 'test' -- "$cur") )
      elif [[ "$cur" == -* ]]; then
        COMPREPLY=( $(compgen -W '--url --timeout --select' -- "$cur") )
      fi
      ;;
    tun)
      if (( COMP_CWORD == 2 )); then
        COMPREPLY=( $(compgen -W 'status on off' -- "$cur") )
      fi
      ;;
    proxy)
      if (( COMP_CWORD == 2 )); then
        COMPREPLY=( $(compgen -W 'status on off env noenv shell-init terminal system' -- "$cur") )
      elif [[ "$prev" == "env" || "$prev" == "noenv" || "$prev" == "shell-init" ]]; then
        COMPREPLY=( $(compgen -W 'bash zsh fish' -- "$cur") )
      elif [[ "$prev" == "terminal" || "$prev" == "system" ]]; then
        COMPREPLY=( $(compgen -W 'status on off' -- "$cur") )
      fi
      ;;
    completion|comp)
      if (( COMP_CWORD == 2 )); then
        COMPREPLY=( $(compgen -W 'generate' -- "$cur") )
      elif (( COMP_CWORD == 3 )); then
        COMPREPLY=( $(compgen -W 'bash zsh fish' -- "$cur") )
      fi
      ;;
  esac
}
complete -F _tz_complete tz
"#
}

fn zsh_script() -> &'static str {
    r#"#compdef tz

_arguments '1:command:(init status st start on stop off end restart r list select node tun proxy setting set profile p core c config cfg completion comp)' \
  '*::arg:->args'

case "$words[2]" in
  profile|p)
    _arguments '1:action:(add a list l info i use u update up remove rm)' \
      '*:options:(--family --all)'
    ;;
  core|c)
    _arguments '1:action:(add a list l info i use u remove rm)'
    ;;
  setting|set)
    _arguments '1:action:(list get set reset)'
    ;;
  config|cfg)
    _arguments '1:action:(build check show)'
    ;;
  node)
    _arguments '1:action:(test)' '*:options:(--url --timeout --select)'
    ;;
  tun)
    _arguments '1:action:(status on off)'
    ;;
  proxy)
    _arguments '1:action:(status on off env noenv shell-init terminal system)' \
      '2:value:(status on off bash zsh fish)'
    ;;
  completion|comp)
    _arguments '1:action:(generate)' '2:shell:(bash zsh fish)'
    ;;
esac
"#
}

fn fish_script() -> &'static str {
    r#"complete -c tz -f -n '__fish_use_subcommand' -a 'init status st start on stop off end restart r list select node tun proxy setting set profile p core c config cfg completion comp'
complete -c tz -s l -l list -r -a '(__fish_print_minimal)' -d 'list or search nodes'
complete -c tz -f -n '__fish_seen_subcommand_from profile p' -a 'add a list l info i use u update up remove rm'
complete -c tz -l family -f -n '__fish_seen_subcommand_from profile p' -a 'clash sing-box'
complete -c tz -l all -f -n '__fish_seen_subcommand_from profile p; and __fish_seen_subcommand_from list l'
complete -c tz -f -n '__fish_seen_subcommand_from core c' -a 'add a list l info i use u remove rm'
complete -c tz -f -n '__fish_seen_subcommand_from setting set' -a 'list get set reset'
complete -c tz -f -n '__fish_seen_subcommand_from config cfg' -a 'build check show'
complete -c tz -f -n '__fish_seen_subcommand_from node' -a 'test'
complete -c tz -l url -l timeout -l select -n '__fish_seen_subcommand_from node test'
complete -c tz -f -n '__fish_seen_subcommand_from tun' -a 'status on off'
complete -c tz -f -n '__fish_seen_subcommand_from proxy' -a 'status on off env noenv shell-init terminal system'
complete -c tz -f -n '__fish_seen_subcommand_from proxy; and __fish_seen_subcommand_from env noenv shell-init' -a 'bash zsh fish'
complete -c tz -f -n '__fish_seen_subcommand_from proxy; and __fish_seen_subcommand_from terminal system' -a 'status on off'
complete -c tz -f -n '__fish_seen_subcommand_from completion comp' -a 'generate'
complete -c tz -f -n '__fish_seen_subcommand_from completion generate' -a 'bash zsh fish'
"#
}
