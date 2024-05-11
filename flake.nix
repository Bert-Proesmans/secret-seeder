{
  inputs = {
    nixpkgs.follows = "nixpkgs-unstable";
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    nixpkgs-stable.url = "github:NixOS/nixpkgs/nixos-23.11";

    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

    rust-overlay.url = "github:oxalica/rust-overlay";
    # Follows currently unused, the overlay output is not bound to the inputs

    nocargo.url = "github:oxalica/nocargo";
    nocargo.inputs.nixpkgs.follows = "nixpkgs";
    nocargo.inputs.registry-crates-io.follows = "registry-crates-io";

    # Manually track crates-io index
    registry-crates-io.url = "github:rust-lang/crates.io-index";
    registry-crates-io.flake = false;
  };

  outputs = { self, ... }@inputs:
    let
      # Simple shortcut
      # At this point no additional functional behaviour required
      lib = inputs.nixpkgs.lib;

      supportedSystems = [ "x86_64-linux" ];

      # Evaluate a new package set with locked in rust binary versions
      rustPkgs = lib.genAttrs supportedSystems (system: ((import inputs.nixpkgs) {
        overlays = [ inputs.rust-overlay.overlays.rust-overlay ];
        localSystem = { inherit system; };
      }));

      # Tool to evaluate configuration with a dedicated pkgs set to each supported system.
      eachSystemWithRust = f: lib.genAttrs supportedSystems (system: (f rustPkgs."${system}"));
      eachSystem = f: lib.genAttrs supportedSystems (system: f system);
    in
    {
      # Format entire flake with;
      # nix fmt
      formatter = eachSystemWithRust (pkgs:
        (inputs.treefmt-nix.lib.evalModule pkgs {
          projectRootFile = "flake.nix";

          programs.nixpkgs-fmt.enable = true;
          # Nix cleanup of dead code
          programs.deadnix.enable = true;
          programs.shellcheck.enable = true;
          programs.rustfmt.enable = true;
        }).config.build.wrapper);

      # Build and start development shell with;
      # nix flake develop
      devShells = eachSystemWithRust (pkgs: {
        default = pkgs.mkShell {
          name = "secret-seeder development";

          # Software required at build-time of shell
          nativeBuildInputs = [
            # Build the formatter software configuration files
            self.outputs.formatter."${pkgs.system}"
          ];

          # Software directly available inside the developer shell
          packages = [
            pkgs.git
            pkgs.rust-analyzer # from overlay
            # In case nightly is needed, use this;
            # pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default)
            (pkgs.rust-bin.stable.latest.default # from overlay
            .override {
              extensions = [
                # None currently
              ];
            })
          ];
        };
      });

      # The entry API to make Nix derivations from your Rust workspace or package.
      # The output of it consists of profile names, like `release` or `dev`, each of which is
      # a attrset of all member package derivations keyed by their package names.
      #
      # By default, the profile names release and dev are configured for all crates in the workspace.
      # # -> { <profile-name> = { <member-pkg-name> = <drv>; }; }
      rustWorkspace = eachSystemWithRust (pkgs: (inputs.nocargo.lib."${pkgs.system}".mkRustPackageOrWorkspace {
        # The root directory, which contains `Cargo.lock` and top-level `Cargo.toml`
        # (the one containing `[workspace]` for workspace).
        src = ./.;

        # Default is rustc from nixpkgs. Overriden here is the latest stable from the overlay.
        rustc = pkgs.rust-bin.stable.latest.default;
      }));

      # Build software with;
      # nix build .#<crate-name>
      packages = eachSystem (system: { }
        // self.outputs.rustWorkspace."${system}".release
        #// (lib.mapAttrs' (name: value: lib.nameValuePair "${name}-dev" value) self.outputs.rustWorkspace."${system}".dev)
      );
    };
}
