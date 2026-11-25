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
        voting = {
          roster_path = cfg.settings.voting.rosterPath;
          internal_quorum = cfg.settings.voting.internalQuorum;
          coalition_quorum = cfg.settings.voting.coalitionQuorum;
          window_hours = cfg.settings.voting.windowHours;
          unaffiliated_weight = cfg.settings.voting.unaffiliatedWeight;
          abstentions = cfg.settings.voting.abstentions;
          exhausted_ballots = cfg.settings.voting.exhaustedBallots;
          last_two_options = cfg.settings.voting.lastTwoOptions;
          rule_changes = cfg.settings.voting.ruleChanges;
        };
      };

      command = "${cfg.package}/bin/kairi --config ${cfg.tomlFile} --data-dir ${cfg.dataDir}";
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
              voting = lib.mkOption {
                default = { };
                type = lib.types.submodule {
                  options = {
                    rosterPath = lib.mkOption {
                      type = lib.types.oneOf [
                        lib.types.str
                        lib.types.path
                      ];
                      default = "";
                      description = ''
                        JSON of the members and groups of the inventory, as
                        `{ members = { ... }; groups = { ... }; }`.
                        Votes can't be counted without it
                      '';
                    };
                    internalQuorum = lib.mkOption {
                      type = lib.types.numbers.between 0 1;
                      default = 0.5;
                      description = "Share of an organisation's roster that has to vote for it to count";
                    };
                    coalitionQuorum = lib.mkOption {
                      type = lib.types.numbers.between 0 1;
                      default = 0.5;
                      description = "Share of the organisations that have to count for a vote to stand";
                    };
                    windowHours = lib.mkOption {
                      type = lib.types.ints.positive;
                      default = 24;
                      description = "How long a vote lasts when its first mail doesn't say";
                    };
                    unaffiliatedWeight = lib.mkOption {
                      type = lib.types.enum [
                        "voters"
                        "roster"
                      ];
                      default = "voters";
                      description = "Size of the unaffiliated block";
                    };
                    abstentions = lib.mkOption {
                      type = lib.types.enum [
                        "counted"
                        "quorum_only"
                        "ignored"
                      ];
                      default = "counted";
                      description = ''
                        Whether the weight of a member who abstains is part of what an option needs
                        more than half of, only counts for the quorum, or is ignored
                      '';
                    };
                    exhaustedBallots = lib.mkOption {
                      type = lib.types.enum [
                        "counted"
                        "dropped"
                      ];
                      default = "counted";
                      description = ''
                        Whether a ballot stays part of what an option needs more than half of once
                        every option it ranked is out of the runoff
                      '';
                    };
                    lastTwoOptions = lib.mkOption {
                      type = lib.types.enum [
                        "runoff"
                        "final"
                      ];
                      default = "runoff";
                      description = ''
                        Whether the weaker of the last two options of a runoff is still eliminated,
                        its ballots passing the other, or the count stops there without a majority
                      '';
                    };
                    ruleChanges = lib.mkOption {
                      type = lib.types.bool;
                      default = false;
                      description = ''
                        Whether `/voteconfig` can change the voting rules until the bot restarts, for
                        a test bot. It only shows them otherwise
                      '';
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
          wants = [ "network-online.target" ];
          after = [ "network-online.target" ];

          serviceConfig = {
            ExecStart = command;
            ExecStartPre = testCommand;
            Restart = "on-failure";
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
