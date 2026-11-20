using System.Threading.Tasks;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The map's still layer over the scenes the window draws, read from the
/// recorded samples (review W1 of #2215). The tick's decision takes the
/// scene, so a Private AI scene, which has no pile, can never be read as a
/// pile crossing between the layers.
/// </summary>
public class StillLayerTests
{
    private static async Task<FlowMapScene> PrivateAIScene()
    {
        MapSnapshot snapshot = await FlowMapSceneTests.Read("normalDay");
        FlowMapScene scene = FlowMapScene.PrivateAI(
            snapshot.Harnesses, "DEST", snapshot.Destinations?.PrivateAi ?? snapshot.Status?.PrivateInferenceState,
            row => "s:" + row.Id, FlowMapSceneTests.ProbeState, FlowMapCopy.Table, snapshot.Destinations);
        Assert.True(scene.DrawsMap);
        Assert.Null(scene.Stack);
        return scene;
    }

    private static async Task<FlowMapScene> TracesScene()
    {
        MapSnapshot snapshot = await FlowMapSceneTests.Read("normalDay");
        FlowMapScene scene = FlowMapScene.Traces(
            snapshot.Tree(MapFeed.CoreScansWhenUnset), snapshot.Gate, FlowMapCopy.Table, snapshot.Contributions);
        Assert.True(scene.DrawsMap);
        Assert.NotNull(scene.Stack);
        return scene;
    }

    [Fact]
    public async Task APrivateAIMapIsNotDrawnAgainOnEveryTick()
    {
        FlowMapScene scene = await PrivateAIScene();
        var still = new StillLayer();
        still.Drawn(scene, pileMoving: false);
        for (int tick = 0; tick < 30; tick++)
        {
            Assert.False(still.Tick(scene, fieldMoves: false, pileMoving: false), $"tick {tick}");
        }
    }

    [Fact]
    public async Task ATracesPileIsDrawnAgainOnlyAsItCrossesBetweenTheLayers()
    {
        FlowMapScene scene = await TracesScene();
        var still = new StillLayer();

        // At rest in the still layer: nothing to draw again.
        still.Drawn(scene, pileMoving: false);
        Assert.False(still.Tick(scene, fieldMoves: false, pileMoving: false));

        // It starts moving: the still layer is drawn once without it.
        Assert.True(still.Tick(scene, fieldMoves: false, pileMoving: true));
        still.Drawn(scene, pileMoving: true);
        Assert.False(still.Tick(scene, fieldMoves: false, pileMoving: true));

        // It comes to rest: drawn once more, with it.
        Assert.True(still.Tick(scene, fieldMoves: false, pileMoving: false));
        still.Drawn(scene, pileMoving: false);
        Assert.False(still.Tick(scene, fieldMoves: false, pileMoving: false));
    }

    [Fact]
    public async Task AShimmerPassAndTheTickAfterItDrawTheFieldAgain()
    {
        FlowMapScene scene = await PrivateAIScene();
        var still = new StillLayer();
        still.Drawn(scene, pileMoving: false);
        Assert.True(still.Tick(scene, fieldMoves: true, pileMoving: false));
        Assert.True(still.Tick(scene, fieldMoves: false, pileMoving: false));
        Assert.False(still.Tick(scene, fieldMoves: false, pileMoving: false));
    }
}
