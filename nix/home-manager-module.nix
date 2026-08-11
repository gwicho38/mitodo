{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.programs.mitodo;
  settingsFormat = pkgs.formats.toml { };
  configFile = settingsFormat.generate "config.toml" cfg.settings;
in {
  meta.maintainers = [ "gwicho38" ];

  options.programs.mitodo = {
    enable = mkEnableOption "mitodo, a TUI todo tracker over plain markdown checklists";

    package = mkOption {
      type = types.package;
      default = pkgs.mitodo;
      defaultText = literalExpression "pkgs.mitodo";
      description = "The mitodo package to use.";
    };

    settings = mkOption {
      type = settingsFormat.type;
      default = { };
      example = literalExpression ''
        {
          refresh_fps = 60;
          article_scope = "unread";
          read_icon = "󰄬";
          unread_icon = "󰄱";
          
          theme = {
            color_palette = {
              background = "#1e1e2e";
              foreground = "#cdd6f4";
              accent_primary = "#f5c2e7";
            };
          };
          
          input_config = {
            scroll_amount = 10;
            mappings = {
              "q" = "quit";
              "j" = "down";
              "k" = "up";
            };
          };
        }
      '';
      description = ''
        Configuration written to {file}`$XDG_CONFIG_HOME/mitodo/config.toml`.
        
        See <https://github.com/gwicho38/mitodo#configuration-options>
        for the full list of options.
      '';
    };
  };

  config = mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile."mitodo/config.toml" = mkIf (cfg.settings != { }) {
      source = configFile;
    };
  };
}
