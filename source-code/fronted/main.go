package main

import (
	"bufio"
	"embed"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/bubbles/progress"
	"github.com/charmbracelet/bubbles/viewport"
	"github.com/charmbracelet/lipgloss"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
)

var (
	backendPath string
	//go:embed docs.md
	docsContent embed.FS
)

func init() {
	home, _ := os.UserHomeDir()
	backendDir := filepath.Join(home, ".ulb")
	os.MkdirAll(backendDir, 0755)
	backendPath = filepath.Join(backendDir, "backend")
}

var rootCmd = &cobra.Command{
	Use:   "ulb",
	Short: "Universal Live Builder - Tool for building custom live ISOs",
	Long: `ULB is a versatile tool that allows users to build customized live ISO images for various Linux distributions like Fedora and Debian. It uses containerization for reproducible builds.`,
}

var cleanCmd = &cobra.Command{
	Use:   "clean",
	Short: "Clean the build cache",
	RunE: func(cmd *cobra.Command, args []string) error {
		return runBackend("clean", "", false)
	},
}

var buildCmd = &cobra.Command{
	Use:   "build",
	Short: "Build the ISO image",
	RunE: func(cmd *cobra.Command, args []string) error {
		release, _ := cmd.Flags().GetBool("release")
		arg := ""
		if release {
			arg = "--release"
		}
		return runBackendWithProgress("build", arg)
	},
}

var initCmd = &cobra.Command{
	Use:   "init",
	Short: "Initialize a new project skeleton",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Create directories
		dirs := []string{"files", "install-files", "scripts", "repos", "build/release", "build/.cache"}
		for _, d := range dirs {
			os.MkdirAll(d, 0755)
		}
		// Create example Config.toml
		configContent := `
# ULB Configuration File
# distro: The base distribution (fedora or debian)
# image_name: Name of the output ISO
# installer: Optional installer package (e.g., anaconda for fedora)
# architecture: Optional architecture (e.g., x86_64)

distro = "fedora"
image_name = "my-live-iso"
installer = "anaconda" # Optional
architecture = "x86_64" # Optional
`
		os.WriteFile("Config.toml", []byte(configContent), 0644)
		// Example package-lists file
		packageListsContent := `# Package Lists
# One package per line
base-system
kernel
`
		os.WriteFile("package-lists", []byte(packageListsContent), 0644)
		// Example packages-lists-remove file
		removeListsContent := `# Packages to Remove
# One package per line
# example-package
`
		os.WriteFile("packages-lists-remove", []byte(removeListsContent), 0644)
		fmt.Println("Project initialized")
		return nil
	},
}

var docsCmd = &cobra.Command{
	Use:   "docs",
	Short: "Display documentation in TUI",
	RunE: func(cmd *cobra.Command, args []string) error {
		content, err := docsContent.ReadFile("docs.md")
		if err != nil {
			return err
		}
		p := tea.NewProgram(initialViewportModel(string(content)))
		if _, err := p.Run(); err != nil {
			return err
		}
		return nil
	},
}

var updateCmd = &cobra.Command{
	Use:   "update",
	Short: "Update the tool and backend",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Update backend
		url := "https://github.com/user/ulb/releases/latest/download/ulb-backend" // Adjust
		resp, err := http.Get(url)
		if err != nil {
			return err
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			return fmt.Errorf("failed to download backend: status code %d", resp.StatusCode)
		}
		out, err := os.Create(backendPath + ".new")
		if err != nil {
			return err
		}
		defer out.Close()
		io.Copy(out, resp.Body)
		os.Rename(backendPath + ".new", backendPath)
		os.Chmod(backendPath, 0755)

		// Update self
		selfUrl := "https://github.com/user/ulb/releases/latest/download/ulb"
		selfResp, err := http.Get(selfUrl)
		if err != nil {
			return err
		}
		defer selfResp.Body.Close()
		if selfResp.StatusCode != http.StatusOK {
			return fmt.Errorf("failed to download ulb: status code %d", selfResp.StatusCode)
		}
		selfPath, _ := os.Executable()
		selfOut, err := os.Create(selfPath + ".new")
		if err != nil {
			return err
		}
		defer selfOut.Close()
		io.Copy(selfOut, selfResp.Body)
		os.Rename(selfPath + ".new", selfPath)
		os.Chmod(selfPath, 0755)
		fmt.Println("Backend and self updated. Restart to apply.")
		return nil
	},
}

var statusCmd = &cobra.Command{
	Use:   "status",
	Short: "Show status of configuration and backend",
	RunE: func(cmd *cobra.Command, args []string) error {
		return runBackend("status", "", false)
	},
}

func main() {
	buildCmd.Flags().BoolP("release", "r", false, "Build release ISO")
	rootCmd.AddCommand(cleanCmd, buildCmd, initCmd, docsCmd, updateCmd, statusCmd)
	rootCmd.Execute()
}

func runBackend(command string, arg string, jsonOutput bool) error {
	if err := validateConfig(); err != nil {
		return err
	}
	cmdArgs := []string{command}
	if arg != "" {
		cmdArgs = append(cmdArgs, arg)
	}
	if jsonOutput {
		cmdArgs = append(cmdArgs, "--json-output")
	}
	cmdArgs = append(cmdArgs, "Config.toml")
	cmd := exec.Command(backendPath, cmdArgs...)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

func runBackendWithProgress(command string, arg string) error {
	if err := validateConfig(); err != nil {
		return err
	}
	cmdArgs := []string{command}
	if arg != "" {
		cmdArgs = append(cmdArgs, arg)
	}
	cmdArgs = append(cmdArgs, "--json-output", "Config.toml")
	cmd := exec.Command(backendPath, cmdArgs...)
	stdout, _ := cmd.StdoutPipe()
	cmd.Stderr = os.Stderr
	if err := cmd.Start(); err != nil {
		return err
	}
	p := progress.New(progress.WithDefaultGradient())
	m := progressModel{progress: p, reader: bufio.NewReader(stdout)}
	program := tea.NewProgram(m)

	programDone := make(chan struct{})
	go func() {
		if _, err := program.Run(); err != nil {
			fmt.Println("Error running program:", err)
		}
		close(programDone)
	}()

	backendErr := cmd.Wait()
	program.Send(quitMsg{})

	<-programDone // Wait for TUI to quit

	return backendErr
}

type progressModel struct {
	progress progress.Model
	reader   *bufio.Reader
	stage    string
}

type progressMsg struct {
	Stage    string
	Progress float64
}

type quitMsg struct{}

func (m progressModel) Init() tea.Cmd {
	return m.readProgress()
}

func (m progressModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case quitMsg:
		return m, tea.Quit
	case tea.KeyMsg:
		if msg.String() == "q" {
			return m, tea.Quit
		}
	case progressMsg:
		m.stage = msg.Stage
		return m, m.progress.SetPercent(msg.Progress)
	case tea.WindowSizeMsg:
		m.progress.Width = msg.Width - 4
		return m, nil
	case progress.FrameMsg:
		newModel, cmd := m.progress.Update(msg)
		if newModel, ok := newModel.(progress.Model); ok {
			m.progress = newModel
		}
		return m, cmd
	}
	return m, m.readProgress()
}

func (m progressModel) View() string {
	return lipgloss.NewStyle().Render(fmt.Sprintf("Stage: %s\n%s", m.stage, m.progress.View()))
}

func (m progressModel) readProgress() tea.Cmd {
	return func() tea.Msg {
		line, err := m.reader.ReadString('\n')
		if err != nil {
			return quitMsg{}
		}
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(line), &data); err != nil {
			return quitMsg{}
		}
		stage, ok := data["stage"].(string)
		if !ok {
			return quitMsg{}
		}
		prog, ok := data["progress"].(float64)
		if !ok {
			return quitMsg{}
		}
		return progressMsg{Stage: stage, Progress: prog}
	}
}

func validateConfig() error {
	viper.SetConfigName("Config")
	viper.SetConfigType("toml")
	viper.AddConfigPath(".")
	if err := viper.ReadInConfig(); err != nil {
		return err
	}
	// Validate required fields
	if viper.GetString("distro") == "" {
		return fmt.Errorf("distro is required")
	}
	if viper.GetString("image_name") == "" {
		return fmt.Errorf("image_name is required")
	}
	return nil
}

// Viewport for docs
type viewportModel struct {
	viewport viewport.Model
	content  string
}

func initialViewportModel(content string) viewportModel {
	vp := viewport.New(78, 20)
	vp.SetContent(content)
	return viewportModel{viewport: vp, content: content}
}

func (m viewportModel) Init() tea.Cmd {
	return nil
}

func (m viewportModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.String() {
		case "q", "esc":
			return m, tea.Quit
		case "up":
			m.viewport.LineUp(1)
		case "down":
			m.viewport.LineDown(1)
		}
	case tea.WindowSizeMsg:
		m.viewport.Width = msg.Width
		m.viewport.Height = msg.Height
	}
	var cmd tea.Cmd
	m.viewport, cmd = m.viewport.Update(msg)
	return m, cmd
}

func (m viewportModel) View() string {
	return m.viewport.View()
}
