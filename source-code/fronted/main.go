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
	"strings"

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
	Short: "Universal Live Builder",
}

var cleanCmd = &cobra.Command{
	Use:   "clean",
	Short: "Clean cache",
	RunE: func(cmd *cobra.Command, args []string) error {
		return runBackend("clean", "", false)
	},
}

var buildCmd = &cobra.Command{
	Use:   "build",
	Short: "Build ISO",
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
	Short: "Initialize project skeleton",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Create directories
		dirs := []string{"files", "install-files", "scripts", "package-lists", "packages-lists-remove", "repos", "build/release", "build/.cache"}
		for _, d := range dirs {
			os.MkdirAll(d, 0755)
		}

		// Create example Config.toml
		configContent := `
distro = "fedora"
image_name = "my-live-iso"
installer = "anaconda"  # Optional
architecture = "x86_64" # Optional
`
		os.WriteFile("Config.toml", []byte(configContent), 0644)

		// Example package-lists
		os.WriteFile("package-lists", []byte("base-system\nkernel\n"), 0644)

		fmt.Println("Project initialized")
		return nil
	},
}

var docsCmd = &cobra.Command{
	Use:   "docs",
	Short: "Open docs in TUI",
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
	Short: "Update tool and backend",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Placeholder: Download from GitHub Releases
		url := "https://github.com/user/ulb/releases/latest/download/ulb-backend" // Adjust
		resp, err := http.Get(url)
		if err != nil {
			return err
		}
		defer resp.Body.Close()

		out, err := os.Create(backendPath)
		if err != nil {
			return err
		}
		defer out.Close()
		io.Copy(out, resp.Body)
		os.Chmod(backendPath, 0755)

		fmt.Println("Backend updated")
		return nil
	},
}

func main() {
	buildCmd.Flags().BoolP("release", "r", false, "Build release ISO")

	rootCmd.AddCommand(cleanCmd, buildCmd, initCmd, docsCmd, updateCmd)
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

	cmdArgs := []string{command, arg, "--json-output", "Config.toml"}
	cmd := exec.Command(backendPath, cmdArgs...)
	stdout, _ := cmd.StdoutPipe()
	cmd.Stderr = os.Stderr
	cmd.Start()

	p := progress.New(progress.WithDefaultGradient())
	m := progressModel{progress: p, reader: bufio.NewReader(stdout)}

	go func() {
		<-tea.NewProgram(m).Run()
	}()

	return cmd.Wait()
}

type progressModel struct {
	progress progress.Model
	reader   *bufio.Reader
	stage    string
}

func (m progressModel) Init() tea.Cmd {
	return m.readProgress
}

func (m progressModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		if msg.String() == "q" {
			return m, tea.Quit
		}
	case progressMsg:
		m.stage = msg.Stage
		return m, m.progress.SetPercent(msg.Progress)
	case progress.FrameMsg:
		newModel, cmd := m.progress.Update(msg)
		if newModel, ok := newModel.(progress.Model); ok {
			m.progress = newModel
		}
		return m, cmd
	}
	return m, m.readProgress
}

func (m progressModel) View() string {
	return lipgloss.NewStyle().Render(fmt.Sprintf("Stage: %s\n%s", m.stage, m.progress.View()))
}

type progressMsg struct {
	Stage    string
	Progress float64
}

func (m progressModel) readProgress() tea.Msg {
	line, err := m.reader.ReadString('\n')
	if err != nil {
		return tea.Quit
	}
	var data map[string]interface{}
	json.Unmarshal([]byte(line), &data)
	stage := data["stage"].(string)
	progress := data["progress"].(float64)
	return progressMsg{Stage: stage, Progress: progress}
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
	return m, nil
}

func (m viewportModel) View() string {
	return m.viewport.View()
}

// docs.md (embedded)
# Universal Live Builder Docs

## Introduction
...

// Add full docs content here
