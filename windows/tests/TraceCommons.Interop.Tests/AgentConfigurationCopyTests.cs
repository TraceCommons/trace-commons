using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

public sealed class AgentConfigurationCopyTests
{
    [Theory]
    [InlineData(null)]
    [InlineData("{}")]
    [InlineData("{\"agent_configuration\":null}")]
    [InlineData("{\"agent_configuration\":{}}")]
    [InlineData("invalid")]
    public void MissingOrIncompleteHandoffIsHidden(string? json) =>
        Assert.Null(AgentConfigurationCopy.Parse(json));

    [Fact]
    public void PromptIsPreservedVerbatim()
    {
        const string prompt = "Configure my app.\nSettings: [my preferences]\nDo not submit traces.";
        var json = JsonSerializer.Serialize(new { agent_configuration = new {
            action = "Open", title = "Title", body = "Body", prerequisite = "Prerequisite",
            prompt, copy = "Copy", copied = "Copied", close = "Close",
        }});
        var copy = Assert.IsType<AgentConfigurationCopy>(AgentConfigurationCopy.Parse(json));
        Assert.Equal(prompt, copy.Prompt);
    }
}
