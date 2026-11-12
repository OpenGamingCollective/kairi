{ moduleWithSystem, ... }:
{
  flake.nixosModules.default = moduleWithSystem (
    perSystem@{ config, pkgs, ... }:
    nixosArgs@{ lib, ... }:
    let
      cfg = nixosArgs.config.services.kairi;
      settingsFormat = pkgs.formats.toml { };

      tomlSettings = {
        discord = {
          token = cfg.settings.discord.token;
          token_path = cfg.settings.discord.tokenPath;
        };
        typesafe = {
          token = cfg.settings.typesafe.token;
          token_path = cfg.settings.typesafe.tokenPath;
        };
        github = {
          app_id = cfg.settings.github.appId;
          pem_path = cfg.settings.github.pemPath;
        };
      };

      command = "${cfg.package}/bin/kairi --config ${cfg.tomlFile}";
      testCommand = "${command} --test";

    in
    {
      options.services.kairi = {
        enable = lib.mkEnableOption "the kairi bot service";

        package = lib.mkOption {
          type = lib.types.package;
          default = config.packages.default;
          description = "The kairi package to use.";
        };

        user = lib.mkOption {
          type = lib.types.str;
          default = "kairi";
          description = ''
            User under which the kairi service runs
          '';
        };

        group = lib.mkOption {
          type = lib.types.str;
          default = "kairi";
          description = ''
            Group under which the kairi service runs
          '';
        };

        dataDir = lib.mkOption {
          type = lib.types.path;
          default = "/var/lib/kairi";
          description = "State directory for the kairi service";
        };
        settings = lib.mkOption {
          default = { };
          type = lib.types.submodule {
            freeformType = settingsFormat.type;
            options = {
              default = { };
              discord = lib.mkOption {
                type = lib.types.submodule {
                  options = {
                    token = lib.mkOption {
                      type = lib.types.str;
                      default = "";
                    };
                    tokenPath = lib.mkOption {
                      type = lib.types.oneOf [
                        lib.types.str
                        lib.types.path
                      ];
                      default = "";
                    };
                  };
                };
              };
              typesafe = lib.mkOption {
                type = lib.types.submodule {
                  options = {
                    token = lib.mkOption {
                      type = lib.types.str;
                      default = "";
                    };
                    tokenPath = lib.mkOption {
                      type = lib.types.oneOf [
                        lib.types.str
                        lib.types.path
                      ];
                      default = "";
                    };
                  };
                };
              };
              github = lib.mkOption {
                type = lib.types.submodule {
                  options = {
                    appId = lib.mkOption {
                      type = lib.types.int;
                      default = 0;
                    };
                    pemPath = lib.mkOption {
                      type = lib.types.oneOf [
                        lib.types.str
                        lib.types.path
                      ];
                      default = "";
                    };
                  };
                };
              };
            };
          };
        };

        tomlFile = lib.mkOption {
          type = lib.types.path;
          internal = true;
          defaultText = lib.literalMD "generated `kairi.toml`";
          description = ''
            Kairi config file
          '';
        };
      };

      config = lib.mkIf cfg.enable {
        services.kairi.tomlFile = lib.mkDefault (settingsFormat.generate "kairi.toml" tomlSettings);

        users.groups.${cfg.group} = { };
        users.users.${cfg.user} = {
          description = "Kairi service user";
          group = cfg.group;
          home = cfg.dataDir;
          createHome = true;
          isSystemUser = true;
        };

        systemd.services.kairi = {
          description = "Kairi service";

          wantedBy = [ "multi-user.target" ];
          after = [ "network.target" ];

          serviceConfig = {
            ExecStart = command;
            ExecStartPre = testCommand;
            RestartSec = 5;
            User = cfg.user;
            Group = cfg.group;
            UMask = "0077";
            NoNewPrivileges = true;
            WorkingDirectory = cfg.dataDir;
            ReadWritePaths = [ cfg.dataDir ];
            RestrictAddressFamilies = [
              "AF_INET"
              "AF_INET6"
            ];
            PrivateTmp = true;
            PrivateDevices = true;
            ProtectSystem = "strict";
            ProtectHome = "true";
            ProtectClock = true;
            ProtectControlGroups = true;
            ProtectKernelModules = true;
            ProtectKernelTunables = true;
            ProtectKernelLogs = true;
            ProtectProc = "invisible";
            RestrictNamespaces = true;
            RestrictRealtime = true;
            RestrictSUIDSGID = true;
            LockPersonality = true;
            MemoryDenyWriteExecute = true;
          };
        };
      };

    }
  );
}
