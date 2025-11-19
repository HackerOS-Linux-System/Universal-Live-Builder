#!/bin/bash

# Rozszerzony zestaw kolorów dla ładniejszego wyglądu (ANSI escape codes)
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
CYAN='\033[0;36m'
WHITE='\033[1;37m'
ORANGE='\033[0;33m'
PINK='\033[1;35m'
NC='\033[0m' # No Color

# Funkcja do wyświetlania komunikatów z kolorami i ikonami
info() {
    echo -e "${BLUE}[INFO]${NC} ${CYAN}$1${NC}"
}

success() {
    echo -e "${GREEN}[SUCCESS]${NC} ${WHITE}$1${NC}"
}

warning() {
    echo -e "${YELLOW}[WARNING]${NC} ${ORANGE}$1${NC}"
}

error() {
    echo -e "${RED}[ERROR]${NC} ${PINK}$1${NC}"
    exit 1
}

progress() {
    echo -e "${PURPLE}[PROGRESS]${NC} ${YELLOW}$1${NC}"
}

header() {
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}$1${NC}"
    echo -e "${GREEN}========================================${NC}"
}

# Wyświetlanie pomocy
show_help() {
    header "Pomoc dla skryptu instalacyjnego ULB"
    echo "Użycie: $0 [opcje]"
    echo ""
    echo "Opcje:"
    echo "  --help          Wyświetla tę pomoc"
    echo "  --force         Wymusza instalację nawet jeśli coś jest zainstalowane"
    echo "  --no-container  Pomija instalację Podmana/Dockera"
    echo ""
    echo "Skrypt instaluje Universal Live Builder (ULB) z automatycznym wykryciem dystrybucji i wsparciem dla systemów atomowych."
    exit 0
}

# Parsowanie argumentów
FORCE=false
NO_CONTAINER=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --help)
            show_help
            ;;
        --force)
            FORCE=true
            shift
            ;;
        --no-container)
            NO_CONTAINER=true
            shift
            ;;
        *)
            warning "Nieznana opcja: $1"
            show_help
            ;;
    esac
done

# Sprawdzenie, czy skrypt jest uruchamiany jako root
if [ "$EUID" -eq 0 ]; then
    warning "Uruchamianie jako root nie jest zalecane, chyba że konieczne. Kontynuuję..."
fi

# Wykrywanie dystrybucji Linuxa - rozszerzone o więcej dystrybucji, w tym Bazzite, Bluefin, Aurora, Vanilla OS itp.
detect_distro() {
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        DISTRO=$ID
        VERSION_ID=$VERSION_ID
    elif [ -f /usr/lib/os-release ]; then
        . /usr/lib/os-release
        DISTRO=$ID
        VERSION_ID=$VERSION_ID
    else
        DISTRO="unknown"
    fi

    # Rozszerzone mapowanie do rodzin dystrybucji, w tym więcej atomowych i gamingowych
    case "$DISTRO" in
        ubuntu|debian|pop|linuxmint|elementary|kali|parrot|zorin|deepin|vanilla|neon|HackerOS)
            DISTRO_FAMILY="debian"
            ;;
        fedora|rhel|centos|rocky|almalinux|oracle|amazon|silverblue|kinoite|bazzite|bluefin|aurora|ublue)
            DISTRO_FAMILY="redhat"
            ;;
        arch|manjaro|endeavouros|garuda|blackarch|arcolinux|cachyos|chimeraos)
            DISTRO_FAMILY="arch"
            ;;
        opensuse|suse|tumbleweed|leap|microos)
            DISTRO_FAMILY="suse"
            ;;
        nixos)
            DISTRO_FAMILY="nix"
            ;;
        guix)
            DISTRO_FAMILY="guix"
            ;;
        gentoo|funtoo|sabayon|calculate)
            DISTRO_FAMILY="gentoo"
            ;;
        slackware|slax|vectorlinux|salix)
            DISTRO_FAMILY="slackware"
            ;;
        void|artix|adelie|alpine|postmarketos)
            DISTRO_FAMILY="independent"
            ;;
        *)
            DISTRO_FAMILY="unknown"
            ;;
    esac

    # Dodatkowe wykrywanie systemów atomowych/immutable (rozszerzone)
    if [ ! -w /usr/bin ] || [ -f /etc/selinux/config ] && grep -q "SELINUX=enforcing" /etc/selinux/config 2>/dev/null || command -v ostree >/dev/null 2>&1 || command -v rpm-ostree >/dev/null 2>&1; then
        ATOMIC=true
        info "Wykryto system atomowy/immutable (np. Bazzite, Silverblue, NixOS)."
    else
        ATOMIC=false
    fi

    info "Wykryta dystrybucja: $DISTRO (rodzina: $DISTRO_FAMILY, atomowy: $ATOMIC)"
}

# Funkcja do instalacji Podmana lub Dockera - rozszerzone wsparcie, w tym dla atomowych jak Bazzite
install_container_engine() {
    if $NO_CONTAINER; then
        info "Pomijanie instalacji silnika kontenerów na żądanie użytkownika."
        return
    fi

    if command -v podman &> /dev/null && ! $FORCE; then
        success "Podman jest już zainstalowany. Pomijam."
        return
    elif command -v docker &> /dev/null && ! $FORCE; then
        success "Docker jest już zainstalowany. Pomijam."
        return
    fi

    progress "Instaluję silnik kontenerów (preferowany Podman, fallback na Docker)..."

    case "$DISTRO_FAMILY" in
        debian)
            if $ATOMIC; then
                warning "Atomowe dystrybucje Debian (np. Vanilla OS) mogą wymagać specyficznych narzędzi. Próbuję apt..."
            fi
            sudo apt update -y || error "Błąd aktualizacji repozytoriów (apt)."
            if ! sudo apt install -y podman; then
                warning "Nie udało się zainstalować Podmana. Próbuję Dockera..."
                sudo apt install -y docker.io containerd || error "Błąd instalacji Dockera."
            fi
            ;;
        redhat)
            if $ATOMIC; then
                if command -v rpm-ostree &> /dev/null; then
                    sudo rpm-ostree install -y --apply-live podman || sudo rpm-ostree install -y podman || error "Błąd instalacji Podmana (rpm-ostree)."
                    warning "Na systemach atomowych jak Bazzite, zmiany mogą wymagać restartu systemu. Zrestartuj po instalacji."
                elif command -v ujust &> /dev/null; then
                    warning "Wykryto ujust (uBlue/Bazzite). Próbuję ujust install-podman jeśli dostępne."
                    ujust install-podman || error "Błąd ujust. Zainstaluj ręcznie."
                else
                    error "Nie wykryto rpm-ostree ani ujust na atomowym redhat. Zainstaluj ręcznie."
                fi
                if [ $? -ne 0 ]; then
                    warning "Nie udało się zainstalować Podmana. Próbuję Dockera..."
                    sudo rpm-ostree install -y --apply-live docker || sudo rpm-ostree install -y docker || error "Błąd instalacji Dockera."
                fi
            else
                if [[ "$DISTRO" == "fedora" && "$VERSION_ID" -ge 31 ]]; then
                    sudo dnf install -y podman || error "Błąd instalacji Podmana (dnf)."
                else
                    sudo dnf install -y podman || sudo yum install -y podman || error "Błąd instalacji Podmana."
                    if [ $? -ne 0 ]; then
                        warning "Nie udało się zainstalować Podmana. Próbuję Dockera..."
                        sudo dnf install -y docker || sudo yum install -y docker || error "Błąd instalacji Dockera."
                    fi
                fi
            fi
            ;;
        arch)
            if $ATOMIC; then
                warning "Atomowe Arch (np. ChimeraOS) mogą wymagać specyficznych narzędzi. Próbuję pacman..."
            fi
            sudo pacman -Syu --noconfirm podman || error "Błąd instalacji Podmana (pacman)."
            if [ $? -ne 0 ]; then
                warning "Nie udało się zainstalować Podmana. Próbuję Dockera..."
                sudo pacman -Syu --noconfirm docker || error "Błąd instalacji Dockera."
            fi
            ;;
        suse)
            if $ATOMIC; then
                warning "Atomowe SUSE (np. MicroOS) używają transactional-update."
                sudo transactional-update pkg install podman || error "Błąd instalacji Podmana (transactional-update)."
                warning "Zmiany wymagają reboot."
            else
                sudo zypper refresh || error "Błąd odświeżania repozytoriów (zypper)."
                sudo zypper install -y podman || error "Błąd instalacji Podmana."
            fi
            if [ $? -ne 0 ]; then
                warning "Nie udało się zainstalować Podmana. Próbuję Dockera..."
                sudo zypper install -y docker || error "Błąd instalacji Dockera."
            fi
            ;;
        nix)
            if command -v nix-env &> /dev/null; then
                nix-env -iA nixpkgs.podman || error "Błąd instalacji Podmana (nix-env)."
            else
                error "Nix nie jest wykryty. Zainstaluj Podmana ręcznie przez nix."
            fi
            ;;
        guix)
            if command -v guix &> /dev/null; then
                guix install podman || error "Błąd instalacji Podmana (guix)."
            else
                error "Guix nie jest wykryty. Zainstaluj Podmana ręcznie."
            fi
            ;;
        gentoo)
            sudo emerge --ask=n app-containers/podman || error "Błąd instalacji Podmana (emerge)."
            if [ $? -ne 0 ]; then
                warning "Nie udało się zainstalować Podmana. Próbuję Dockera..."
                sudo emerge --ask=n app-containers/docker || error "Błąd instalacji Dockera."
            fi
            ;;
        slackware)
            warning "Slackware wymaga ręcznej instalacji. Pobierz i zainstaluj Podmana/Dockera manualnie."
            error "Brak automatycznego wsparcia dla Slackware."
            ;;
        independent)
            case "$DISTRO" in
                alpine)
                    sudo apk add podman || error "Błąd instalacji Podmana (apk)."
                    ;;
                void)
                    sudo xbps-install -Sy podman || error "Błąd instalacji Podmana (xbps)."
                    ;;
                *)
                    warning "Niezależne dystrybucje wymagają specyficznych komend. Zainstaluj ręcznie."
                    error "Brak automatycznego wsparcia."
                    ;;
            esac
            ;;
        unknown)
            error "Nieznana dystrybucja. Zainstaluj Podmana/Dockera ręcznie."
            ;;
    esac

    # Uruchomienie i włączenie usług jeśli to Docker (Podman jest bezdemonowy)
    if command -v docker &> /dev/null; then
        sudo systemctl enable --now docker || warning "Nie udało się uruchomić usługi Dockera. Sprawdź ręcznie."
        sudo usermod -aG docker $USER || warning "Nie udało się dodać użytkownika do grupy docker."
    elif command -v podman &> /dev/null; then
        info "Podman jest bezdemonowy - nie wymaga usług systemowych."
    fi

    success "Silnik kontenerów zainstalowany pomyślnie."
}

# Główna część skryptu - instalacja ULB
URL="https://github.com/michal92299/Universal-Live-Builder/releases/download/v0.1.0/ulb"
TMP_DIR="/tmp/ulb/download-ulb"
BIN_NAME="ulb"

header "Rozpoczynanie instalacji Universal Live Builder (ULB)"

# Wykryj dystrybucję
detect_distro

# Zainstaluj Podmana/Dockera jeśli potrzeba
install_container_engine

# Utwórz katalog tymczasowy
mkdir -p "$TMP_DIR" || error "Błąd tworzenia katalogu tymczasowego: $TMP_DIR"

# Pobranie pliku z progressem (używając curl z --progress-bar jeśli dostępne)
progress "Pobieranie ULB z $URL..."
curl -L "$URL" -o "$TMP_DIR/$BIN_NAME" --progress-bar || error "Błąd pobierania pliku. Sprawdź połączenie internetowe."

# Nadanie uprawnień
chmod +x "$TMP_DIR/$BIN_NAME" || error "Błąd nadawania uprawnień do wykonywania."

# Instalacja w zależności od typu systemu (rozszerzone o atomowe)
if ! $ATOMIC && [ -w /usr/bin ]; then
    info "System nie jest atomowy → instaluję do /usr/bin"
    sudo mv "$TMP_DIR/$BIN_NAME" /usr/bin/ || error "Błąd przenoszenia do /usr/bin. Uruchom z sudo?"
else
    info "System jest atomowy/immutable → instaluję do ~/.local/bin"
    mkdir -p ~/.local/bin || error "Błąd tworzenia ~/.local/bin."
    mv "$TMP_DIR/$BIN_NAME" ~/.local/bin/ || error "Błąd przenoszenia do ~/.local/bin."
    # Dodaj do PATH jeśli potrzeba
    if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
        echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
        echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc  # Dla zsh jeśli używane
        warning "Dodano ~/.local/bin do PATH. Zrestartuj terminal lub uruchom 'source ~/.bashrc'."
    fi
fi

# Czyszczenie plików tymczasowych
rm -rf "$TMP_DIR" || warning "Nie udało się całkowicie usunąć katalogu tymczasowego."

success "Instalacja ULB zakończona sukcesem! Uruchom 'ulb' aby zacząć."
header "Dziękujemy za użycie skryptu instalacyjnego!"
